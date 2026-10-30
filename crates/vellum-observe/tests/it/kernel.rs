// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The recorder installed through the kernel's telemetry hook.

use std::sync::Arc;

use vellum_kernel::telemetry::{TelemetryConfig, subscriber_with};
use vellum_observe::{TraceQuery, TraceStore, recorder_layer};

#[test]
fn kernel_subscriber_feeds_the_ring_buffer() {
    let store = Arc::new(TraceStore::default());
    let config = TelemetryConfig::from_lookup("vellum-test", "0.0.0", |_| None);
    let (subscriber, _guard) = subscriber_with(
        &config,
        Some(Box::new(recorder_layer(Arc::clone(&store)))),
    )
    .unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!(target: "vellum_jobs", "job.digest");
        let _entered = span.enter();
        tracing::info!("sent");
    });
    let rows = store.query(&TraceQuery::default());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().map(|r| r.name.as_str()), Some("job.digest"));
}
