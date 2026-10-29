// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Request spans and the `Server-Timing` header.
//!
//! The call shape matches `vellum-observe` (begin, collect, finish) so the
//! gateway switches to it by changing one import once that crate lands.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::future::Future;
use std::time::{Duration, Instant};

use tracing::Span;

tokio::task_local! {
    static TIMINGS: RefCell<Vec<(&'static str, Duration)>>;
}

/// Entries recorded while a request ran.
#[derive(Debug, Default, Clone)]
pub struct Timings(Vec<(&'static str, Duration)>);

/// Adds a `Server-Timing` entry (`db`, `authz`, `render`) to the current
/// request. Outside a request it does nothing.
pub fn record(name: &'static str, duration: Duration) {
    let _ = TIMINGS.try_with(|t| t.borrow_mut().push((name, duration)));
}

/// Runs `fut`, collecting the timings it records.
pub async fn collect<F: Future>(fut: F) -> (F::Output, Timings) {
    TIMINGS
        .scope(RefCell::new(Vec::new()), async move {
            let out = fut.await;
            let timings = TIMINGS.with(RefCell::take);
            (out, Timings(timings))
        })
        .await
}

/// One request being observed.
#[derive(Debug)]
pub struct RequestObservation {
    span: Span,
    started: Instant,
}

/// What finishing a request yields.
#[derive(Debug)]
pub struct FinishedRequest {
    /// The full `Server-Timing` header value.
    pub server_timing: String,
}

impl RequestObservation {
    /// Starts the root span for a request on `route` (the matched template,
    /// or `unmatched`).
    #[must_use]
    pub fn begin(method: &str, route: &str, request_id: &str) -> Self {
        let span = tracing::info_span!(
            target: "vellum_api",
            "http.request",
            request_id,
            http.request.method = method,
            http.route = route,
            http.response.status_code = tracing::field::Empty,
        );
        Self {
            span,
            started: Instant::now(),
        }
    }

    /// The span to instrument the request with.
    #[must_use]
    pub const fn span(&self) -> &Span {
        &self.span
    }

    /// Records the status and builds the `Server-Timing` value.
    #[must_use]
    pub fn finish(self, status: u16, timings: &Timings) -> FinishedRequest {
        let _ = self.span.record("http.response.status_code", status);
        let total = self.started.elapsed();
        let mut header = format!("total;dur={:.1}", ms(total));
        for (name, duration) in &timings.0 {
            let _ = write!(header, ", {name};dur={:.1}", ms(*duration));
        }
        FinishedRequest {
            server_timing: header,
        }
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collects_entries() {
        let ((), timings) = collect(async {
            record("db", Duration::from_millis(3));
            record("authz", Duration::from_micros(500));
        })
        .await;
        let obs = RequestObservation::begin("GET", "/api/v1/x", "r");
        let done = obs.finish(200, &timings);
        assert!(done.server_timing.starts_with("total;dur="));
        assert!(done.server_timing.contains(", db;dur=3.0"));
        assert!(done.server_timing.contains(", authz;dur=0.5"));
        record("outside", Duration::ZERO);
    }
}
