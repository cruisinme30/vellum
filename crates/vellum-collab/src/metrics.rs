// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! OpenTelemetry instruments for rooms (exported by
//! `vellum-kernel::telemetry`).

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};

/// Every instrument the collab service records.
#[derive(Debug, Clone)]
pub struct Metrics {
    pub rooms: UpDownCounter<i64>,
    pub peers: UpDownCounter<i64>,
    pub updates: Counter<u64>,
    pub update_bytes: Histogram<u64>,
    pub apply_ms: Histogram<f64>,
    pub rejected: Counter<u64>,
    pub dropped_peers: Counter<u64>,
    pub persist_ms: Histogram<f64>,
    pub persist_errors: Counter<u64>,
    pub versions: Counter<u64>,
    pub load_ms: Histogram<f64>,
}

impl Metrics {
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("vellum-collab");
        Self {
            rooms: meter
                .i64_up_down_counter("vellum.collab.rooms")
                .with_description("Open collaboration rooms")
                .build(),
            peers: meter
                .i64_up_down_counter("vellum.collab.peers")
                .with_description("Connected editors")
                .build(),
            updates: meter
                .u64_counter("vellum.collab.updates")
                .with_description("Document updates applied")
                .build(),
            update_bytes: meter
                .u64_histogram("vellum.collab.update.size")
                .with_unit("By")
                .with_description("Size of applied document updates")
                .build(),
            apply_ms: meter
                .f64_histogram("vellum.collab.apply.duration")
                .with_unit("ms")
                .with_description("Time to apply and fan out one message")
                .build(),
            rejected: meter
                .u64_counter("vellum.collab.rejected")
                .with_description(
                    "Messages refused (read-only, malformed, too large)",
                )
                .build(),
            dropped_peers: meter
                .u64_counter("vellum.collab.peers.dropped")
                .with_description("Editors disconnected by the server")
                .build(),
            persist_ms: meter
                .f64_histogram("vellum.collab.persist.duration")
                .with_unit("ms")
                .with_description("Time to append one batch to the update log")
                .build(),
            persist_errors: meter
                .u64_counter("vellum.collab.persist.errors")
                .with_description("Failed storage writes (retried)")
                .build(),
            versions: meter
                .u64_counter("vellum.collab.versions")
                .with_description("Markdown versions written")
                .build(),
            load_ms: meter
                .f64_histogram("vellum.collab.room.load.duration")
                .with_unit("ms")
                .with_description("Time to re-create a room from storage")
                .build(),
        }
    }
}

/// A one-attribute label set.
pub fn label(key: &'static str, value: &'static str) -> [KeyValue; 1] {
    [KeyValue::new(key, value)]
}
