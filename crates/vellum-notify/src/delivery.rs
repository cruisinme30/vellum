// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Delivering requests: the transport port, retries and endpoint health.
//!
//! Deliveries are queued as jobs (Postgres `SKIP LOCKED` or Redis). Each
//! attempt calls [`attempt`], which sends through an [`HttpTransport`] and
//! classifies the response into an [`Outcome`]. The job runner then asks
//! the [`RetryPolicy`] when to try again and updates [`EndpointHealth`],
//! which pauses a subscription that keeps failing (D80).
//!
//! Metrics are emitted as `tracing` events that the kernel's telemetry
//! layer turns into OpenTelemetry instruments:
//! `vellum.notify.deliveries` (counter, by channel and outcome) and
//! `vellum.notify.delivery_ms` (histogram).

use std::future::Future;
use std::time::Instant;

use jiff::{SignedDuration, Timestamp};
use tracing::Instrument as _;

use crate::webhook::OutboundRequest;

/// What the receiver answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The `Retry-After` header in seconds, when present.
    pub retry_after: Option<u64>,
}

/// Why a request never got a response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The connection or the whole request timed out (10 s budget).
    #[error("request timed out")]
    Timeout,
    /// The host resolved to a refused address (see [`crate::endpoint`]).
    #[error("refused by the SSRF guard: {0}")]
    Refused(String),
    /// DNS, TCP or TLS failed.
    #[error("connection failed: {0}")]
    Connect(String),
}

/// Sends HTTP requests. Implemented in the edge layer over the shared HTTP
/// client, which re-checks resolved addresses with
/// [`crate::endpoint::check_resolved`] and applies a 10 s timeout.
pub trait HttpTransport: Send + Sync {
    /// Posts the request and returns the response status.
    fn post(
        &self,
        request: &OutboundRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;
}

/// The classified result of one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 2xx: done.
    Delivered,
    /// Worth retrying: 408, 429, 5xx, timeouts and connection failures.
    /// Carries the receiver's `Retry-After`, if any.
    Retry(Option<u64>),
    /// Other 4xx (or a refused address): retrying will not help.
    Rejected,
    /// 410 Gone: the receiver asked to be unsubscribed.
    Gone,
}

impl Outcome {
    /// Classifies a transport result.
    #[must_use]
    pub const fn classify(result: &Result<HttpResponse, TransportError>) -> Self {
        match result {
            Ok(response) => match response.status {
                200..=299 => Self::Delivered,
                410 => Self::Gone,
                408 | 429 | 500..=599 => Self::Retry(response.retry_after),
                _ => Self::Rejected,
            },
            Err(TransportError::Refused(_)) => Self::Rejected,
            Err(TransportError::Timeout | TransportError::Connect(_)) => Self::Retry(None),
        }
    }

    /// The metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Retry(_) => "retry",
            Self::Rejected => "rejected",
            Self::Gone => "gone",
        }
    }
}

/// When to try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Delay before each retry; the length is the number of retries.
    pub delays: Vec<SignedDuration>,
    /// The longest `Retry-After` honoured.
    pub max_retry_after: SignedDuration,
}

impl Default for RetryPolicy {
    /// Eight attempts over about 45 hours: now, 1 m, 5 m, 30 m, 2 h, 6 h,
    /// 12 h and 24 h later.
    fn default() -> Self {
        Self {
            delays: vec![
                SignedDuration::from_mins(1),
                SignedDuration::from_mins(5),
                SignedDuration::from_mins(30),
                SignedDuration::from_hours(2),
                SignedDuration::from_hours(6),
                SignedDuration::from_hours(12),
                SignedDuration::from_hours(24),
            ],
            max_retry_after: SignedDuration::from_hours(1),
        }
    }
}

impl RetryPolicy {
    /// When attempt number `attempt` (0 for the first) should be followed
    /// by another, or `None` when the outcome is final or retries ran out.
    #[must_use]
    pub fn next_attempt(
        &self,
        attempt: usize,
        outcome: Outcome,
        now: Timestamp,
    ) -> Option<Timestamp> {
        let Outcome::Retry(retry_after) = outcome else {
            return None;
        };
        let scheduled = *self.delays.get(attempt)?;
        let asked = retry_after
            .and_then(|seconds| i64::try_from(seconds).ok())
            .map(SignedDuration::from_secs)
            .map_or(SignedDuration::ZERO, |d| d.min(self.max_retry_after));
        now.checked_add(scheduled.max(asked)).ok()
    }
}

/// Failure tracking for one subscription or push endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EndpointHealth {
    /// Deliveries that finally failed since the last success.
    pub consecutive_failures: u32,
    /// When the first of those failures happened.
    pub failing_since: Option<Timestamp>,
}

/// What to do with the endpoint after a final outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAction {
    /// Keep it.
    Keep,
    /// Pause it and email the admins who own it.
    Pause,
    /// Delete it: the receiver answered 410 Gone. Push subscriptions use this.
    Remove,
}

impl EndpointHealth {
    /// Failures in a row before an endpoint can be paused.
    pub const PAUSE_AFTER_FAILURES: u32 = 10;
    /// How long it must have been failing before it is paused.
    pub const PAUSE_AFTER: SignedDuration = SignedDuration::from_hours(72);

    /// Records a delivery's final outcome and says what to do next.
    pub fn record(&mut self, outcome: Outcome, now: Timestamp) -> HealthAction {
        match outcome {
            Outcome::Delivered => {
                *self = Self::default();
                HealthAction::Keep
            }
            Outcome::Gone => HealthAction::Remove,
            Outcome::Retry(_) | Outcome::Rejected => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                let since = *self.failing_since.get_or_insert(now);
                if self.consecutive_failures >= Self::PAUSE_AFTER_FAILURES
                    && now.duration_since(since) >= Self::PAUSE_AFTER
                {
                    HealthAction::Pause
                } else {
                    HealthAction::Keep
                }
            }
        }
    }
}

/// Sends one attempt inside a span and records the delivery metrics.
pub async fn attempt<T: HttpTransport>(
    transport: &T,
    request: &OutboundRequest,
    channel: &'static str,
    attempt_number: usize,
) -> Outcome {
    let span = tracing::info_span!(
        "notify.deliver",
        otel.kind = "client",
        channel,
        delivery_id = %request.delivery_id,
        url.host = request.url.host_str().unwrap_or_default(),
        attempt = attempt_number,
        http.response.status_code = tracing::field::Empty,
        outcome = tracing::field::Empty,
    );
    async {
        let started = Instant::now();
        let result = transport.post(request).await;
        let outcome = Outcome::classify(&result);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let span = tracing::Span::current();
        if let Ok(response) = &result {
            let _ = span.record("http.response.status_code", response.status);
        }
        let _ = span.record("outcome", outcome.as_str());
        if let Err(error) = &result {
            tracing::warn!(%error, "delivery attempt failed");
        }
        tracing::info!(
            monotonic_counter.vellum.notify.deliveries = 1_u64,
            histogram.vellum.notify.delivery_ms = elapsed_ms,
            channel,
            outcome = outcome.as_str(),
        );
        outcome
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16) -> Result<HttpResponse, TransportError> {
        if status == 0 {
            Err(TransportError::Timeout)
        } else {
            Ok(HttpResponse {
                status,
                retry_after: None,
            })
        }
    }

    fn at(hours: i64) -> Timestamp {
        Timestamp::from_second(hours * 3600).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    #[test]
    fn classification() {
        assert_eq!(Outcome::classify(&response(204)), Outcome::Delivered);
        assert_eq!(Outcome::classify(&response(410)), Outcome::Gone);
        assert_eq!(Outcome::classify(&response(404)), Outcome::Rejected);
        assert_eq!(Outcome::classify(&response(503)), Outcome::Retry(None));
        assert_eq!(Outcome::classify(&response(429)), Outcome::Retry(None));
        assert_eq!(Outcome::classify(&response(0)), Outcome::Retry(None));
        assert_eq!(
            Outcome::classify(&Err(TransportError::Refused("10.0.0.1".to_owned()))),
            Outcome::Rejected
        );
    }

    #[test]
    fn retry_schedule_and_retry_after() {
        let policy = RetryPolicy::default();
        let now = at(0);
        assert_eq!(
            policy.next_attempt(0, Outcome::Retry(None), now),
            now.checked_add(SignedDuration::from_mins(1)).ok()
        );
        assert_eq!(
            policy.next_attempt(0, Outcome::Retry(Some(600)), now),
            now.checked_add(SignedDuration::from_mins(10)).ok()
        );
        assert_eq!(
            policy.next_attempt(0, Outcome::Retry(Some(86_400)), now),
            now.checked_add(SignedDuration::from_hours(1)).ok()
        );
        assert_eq!(policy.next_attempt(7, Outcome::Retry(None), now), None);
        assert_eq!(policy.next_attempt(0, Outcome::Rejected, now), None);
        assert_eq!(policy.next_attempt(0, Outcome::Delivered, now), None);
    }

    #[test]
    fn endpoints_pause_after_three_days_of_failures() {
        let mut health = EndpointHealth::default();
        for hour in 0..9 {
            assert_eq!(
                health.record(Outcome::Rejected, at(hour * 10)),
                HealthAction::Keep
            );
        }
        // The tenth failure, 90 hours after the first, pauses the endpoint.
        assert_eq!(
            health.record(Outcome::Rejected, at(90)),
            HealthAction::Pause
        );

        let mut quick = EndpointHealth::default();
        for minute in 0..20 {
            let when = at(0)
                .checked_add(SignedDuration::from_mins(minute))
                .unwrap_or_else(|_| at(0));
            assert_eq!(quick.record(Outcome::Retry(None), when), HealthAction::Keep);
        }
        assert_eq!(quick.record(Outcome::Delivered, at(1)), HealthAction::Keep);
        assert_eq!(quick, EndpointHealth::default());
        assert_eq!(quick.record(Outcome::Gone, at(1)), HealthAction::Remove);
    }

    struct Fixed(u16);

    impl HttpTransport for Fixed {
        async fn post(&self, _request: &OutboundRequest) -> Result<HttpResponse, TransportError> {
            Ok(HttpResponse {
                status: self.0,
                retry_after: None,
            })
        }
    }

    #[tokio::test]
    async fn attempt_classifies_the_transport_result() {
        let request = OutboundRequest {
            url: url::Url::parse("https://hooks.northstar.dev").unwrap_or_else(|_| unreachable!()),
            headers: Vec::new(),
            body: Vec::new(),
            delivery_id: "d".to_owned(),
        };
        assert_eq!(
            attempt(&Fixed(200), &request, "webhook", 0).await,
            Outcome::Delivered
        );
        assert_eq!(
            attempt(&Fixed(502), &request, "webhook", 1).await,
            Outcome::Retry(None)
        );
    }
}
