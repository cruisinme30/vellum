// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Observability for Vellum (brief §12, ADR 0021).
//!
//! Traces, metrics and logs leave the process through the OpenTelemetry
//! pipeline in `vellum_kernel::telemetry`. This crate adds what the single
//! binary needs on top:
//!
//! - [`RequestRecorder`], a `tracing` layer that keeps the last 10 000
//!   traces in a bounded [`TraceStore`] for the Admin → Requests waterfall;
//! - [`RequestObservation`], the hooks the gateway calls per request: W3C
//!   trace context, the root span, RED and saturation metrics per route
//!   ([`RouteMetrics`]) and the `Server-Timing` header ([`server_timing`]);
//! - [`ClientIngest`], which joins browser measurements to server traces;
//! - [`budget`], the p95 latency budget of every route;
//! - [`slo`], the service level objectives and error-budget burn rates;
//! - [`flame`], span-time flame graphs for `vellum profile`;
//! - [`process`], resident memory and CPU time.

pub mod budget;
pub mod client;
pub mod flame;
pub mod process;
pub mod recorder;
pub mod red;
pub mod request;
pub mod server_timing;
pub mod slo;
pub mod store;
mod trace_id;

use std::sync::Arc;

pub use budget::{Breach, Budgets, RouteBudget};
pub use client::{ClientIngest, ClientReport};
pub use recorder::{RecorderConfig, RequestRecorder};
pub use red::{RouteMetrics, RouteStat, UNMATCHED_ROUTE};
pub use request::{FinishedRequest, RequestObservation};
pub use store::{Trace, TraceQuery, TraceStore, TraceSummary};
pub use trace_id::{ParseError, SpanId, TraceId, TraceParent};

/// Serializes the unit tests that install a `tracing` subscriber: tests on
/// other threads that hit the same callsites without one can otherwise
/// leave them cached as uninteresting.
#[cfg(test)]
pub(crate) static TRACING_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The recording layer with default limits, writing into `store`.
///
/// Install it next to the kernel's telemetry layers:
/// `vellum_kernel::telemetry::init_with(&config, Some(Box::new(recorder_layer(store))))`.
#[must_use]
pub fn recorder_layer(store: Arc<TraceStore>) -> RequestRecorder {
    RequestRecorder::new(store, RecorderConfig::default())
}
