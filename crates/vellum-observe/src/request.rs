// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The per-request hooks the gateway middleware calls.
//!
//! The gateway owns the HTTP middleware (request ids, timeouts, the
//! `Server-Timing` header). It calls [`RequestObservation::begin`] when a
//! request arrives and [`RequestObservation::finish`] when the response is
//! ready; everything observability needs happens between the two:
//!
//! 1. the incoming W3C `traceparent` is joined, or a new trace starts;
//! 2. the root `http.request` span carries the fields the Admin → Requests
//!    list shows (route template, method, status, request id, trace id);
//! 3. the route is marked in flight (saturation) and, on finish, its
//!    duration and status are recorded (rate, errors, duration);
//! 4. the `Server-Timing` value is built, including `traceparent`, so the
//!    browser can join its own measurements to the server trace.

use std::time::{Duration, Instant};

use opentelemetry::trace::{SpanContext, TraceContextExt as _, TraceFlags, TraceState};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::red::{InFlight, RouteMetrics};
use crate::server_timing::{self, Metric};
use crate::trace_id::{SpanId, TraceId, TraceParent};

/// The name of the root span of every HTTP request.
pub const REQUEST_SPAN: &str = "http.request";

/// One request between arrival and response.
#[derive(Debug)]
#[must_use = "call `finish` when the response is ready"]
pub struct RequestObservation {
    span: Span,
    start: Instant,
    metrics: RouteMetrics,
    method: String,
    route: String,
    trace: TraceParent,
    in_flight: InFlight,
}

/// What the gateway needs once the request is finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRequest {
    /// The full `Server-Timing` header value.
    pub server_timing: String,
    /// The trace id, for logs and error bodies.
    pub trace_id: TraceId,
    /// How long the request took.
    pub duration: Duration,
}

fn otel_context(trace: &TraceParent) -> opentelemetry::Context {
    let context = SpanContext::new(
        opentelemetry::trace::TraceId::from_bytes(trace.trace_id.to_bytes()),
        opentelemetry::trace::SpanId::from_bytes(trace.parent_id.to_bytes()),
        if trace.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        },
        true,
        TraceState::default(),
    );
    opentelemetry::Context::new().with_remote_span_context(context)
}

impl RequestObservation {
    /// Starts observing a request.
    ///
    /// `route` must be the matched route *template* (such as
    /// `/api/v1/pages/{page_id}`), or [`crate::UNMATCHED_ROUTE`]; never the
    /// raw path, which would make metric labels unbounded.
    pub fn begin(
        metrics: &RouteMetrics,
        method: &str,
        route: &str,
        traceparent: Option<&str>,
        request_id: &str,
    ) -> Self {
        let incoming = traceparent.and_then(|v| v.parse::<TraceParent>().ok());
        let span = tracing::info_span!(
            target: "vellum_api",
            REQUEST_SPAN,
            otel.name = %format!("{method} {route}"),
            otel.kind = "server",
            trace_id = tracing::field::Empty,
            request_id = %request_id,
            http.request.method = %method,
            http.route = %route,
            http.response.status_code = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        if let Some(parent) = &incoming {
            // Fails only when no OpenTelemetry layer is installed, in which
            // case the recorder still uses the incoming trace id below.
            let _ = span.set_parent(otel_context(parent));
        }
        // When OpenTelemetry is exporting, its ids are the source of truth;
        // otherwise keep the caller's trace id or start a new one.
        let otel = span.context().span().span_context().clone();
        let trace = if otel.is_valid() {
            TraceParent {
                trace_id: otel_trace_id(&otel).unwrap_or_else(TraceId::random),
                parent_id: otel_span_id(&otel).unwrap_or_else(SpanId::random),
                sampled: otel.is_sampled(),
            }
        } else {
            incoming.map_or_else(TraceParent::new_root, |p| TraceParent {
                parent_id: SpanId::random(),
                ..p
            })
        };
        let _ = span.record("trace_id", tracing::field::display(trace.trace_id));
        Self {
            in_flight: metrics.start(method, route),
            span,
            start: Instant::now(),
            metrics: metrics.clone(),
            method: method.to_owned(),
            route: route.to_owned(),
            trace,
        }
    }

    /// The root span; instrument the handler future with it.
    #[must_use]
    pub const fn span(&self) -> &Span {
        &self.span
    }

    /// The trace this request belongs to.
    #[must_use]
    pub const fn trace_id(&self) -> TraceId {
        self.trace.trace_id
    }

    /// Records the outcome and builds the `Server-Timing` value from the
    /// metrics collected with [`server_timing::collect`].
    #[must_use]
    pub fn finish(self, status: u16, timings: Vec<Metric>) -> FinishedRequest {
        let duration = self.start.elapsed();
        let _ = self.span.record("http.response.status_code", status);
        if status >= 500 {
            let _ = self.span.record("otel.status_code", "ERROR");
        }
        self.metrics
            .record(&self.method, &self.route, status, duration);
        drop(self.in_flight);
        let mut metrics = timings;
        metrics.push(Metric {
            name: "total".to_owned(),
            duration: Some(duration),
            description: None,
        });
        metrics.push(Metric {
            name: "traceparent".to_owned(),
            duration: None,
            description: Some(self.trace.to_string()),
        });
        FinishedRequest {
            server_timing: server_timing::header_value(&metrics),
            trace_id: self.trace.trace_id,
            duration,
        }
    }
}

fn otel_trace_id(context: &SpanContext) -> Option<TraceId> {
    context.trace_id().to_string().parse().ok()
}

fn otel_span_id(context: &SpanContext) -> Option<SpanId> {
    context.span_id().to_string().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::{RecorderConfig, RequestRecorder};
    use crate::store::{TraceQuery, TraceStore};
    use std::sync::Arc;
    use tracing::Instrument as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    async fn handle(
        metrics: &RouteMetrics,
        traceparent: Option<&str>,
        status: u16,
    ) -> FinishedRequest {
        let obs = RequestObservation::begin(
            metrics,
            "GET",
            "/api/v1/pages/{page_id}",
            traceparent,
            "req-1",
        );
        let span = obs.span().clone();
        let ((), timings) = server_timing::collect(
            async {
                let db = tracing::info_span!("db.query");
                async { server_timing::record("db", Duration::from_millis(2)) }
                    .instrument(db)
                    .await;
            }
            .instrument(span),
        )
        .await;
        obs.finish(status, timings)
    }

    #[tokio::test]
    async fn joins_incoming_trace_and_records_everything() {
        let _lock = crate::TRACING_TEST_LOCK.lock().await;
        let store = Arc::new(TraceStore::default());
        let subscriber = tracing_subscriber::registry().with(RequestRecorder::new(
            Arc::clone(&store),
            RecorderConfig::default(),
        ));
        let _default = tracing::subscriber::set_default(subscriber);
        let metrics = RouteMetrics::new();
        let incoming = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let done = handle(&metrics, Some(incoming), 503).await;
        assert_eq!(
            done.trace_id.to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert!(done.server_timing.starts_with("db;dur=2.0, total;dur="));
        assert!(
            done.server_timing
                .contains("traceparent;desc=\"00-4bf92f3577b34da6a3ce929d0e0e4736-")
        );
        let rows = store.query(&TraceQuery::default());
        let row = rows.first().unwrap();
        assert_eq!(row.trace_id, done.trace_id);
        assert_eq!(row.status, Some(503));
        assert!(row.error);
        assert_eq!(row.request_id.as_deref(), Some("req-1"));
        assert_eq!(row.span_count, 2);
        let stat = metrics.snapshot(1);
        assert_eq!(stat.first().unwrap().counts.errors, 1);
        assert_eq!(metrics.in_flight(), 0);
    }

    #[tokio::test]
    async fn starts_a_new_trace_without_a_header() {
        let _lock = crate::TRACING_TEST_LOCK.lock().await;
        let metrics = RouteMetrics::new();
        let a = handle(&metrics, None, 200).await;
        let b = handle(&metrics, Some("garbage"), 200).await;
        assert_ne!(a.trace_id, b.trace_id);
        assert!(a.duration < Duration::from_secs(5));
    }

    #[test]
    fn otel_ids_convert() {
        let tp = TraceParent::new_root();
        let cx = otel_context(&tp);
        let sc = cx.span().span_context().clone();
        assert_eq!(otel_trace_id(&sc), Some(tp.trace_id));
        assert_eq!(otel_span_id(&sc), Some(tp.parent_id));
        let unsampled = TraceParent {
            sampled: false,
            ..tp
        };
        assert!(!otel_context(&unsampled).span().span_context().is_sampled());
    }
}
