// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Metrics, recorded through the global OpenTelemetry meter that
//! `vellum_kernel::telemetry::init` installs. Spans come from `tracing`.

use std::sync::OnceLock;
use std::time::Instant;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};

/// Counter of rows whose computed values were refreshed.
pub const ROWS_COMPUTED_TOTAL: &str = "vellum_collections_rows_computed_total";
/// Histogram of compute batch latency in seconds.
pub const COMPUTE_SECONDS: &str = "vellum_collections_compute_duration_seconds";
/// Counter of formula cells that evaluated to an error.
pub const FORMULA_ERRORS_TOTAL: &str = "vellum_collections_formula_errors_total";
/// Histogram of view query latency in seconds, labelled `kind`.
pub const QUERY_SECONDS: &str = "vellum_collections_query_duration_seconds";
/// Counter of CSV rows imported or exported, labelled `direction`.
pub const CSV_ROWS_TOTAL: &str = "vellum_collections_csv_rows_total";

struct Instruments {
    rows: Counter<u64>,
    compute: Histogram<f64>,
    formula_errors: Counter<u64>,
    query: Histogram<f64>,
    csv_rows: Counter<u64>,
}

fn instruments() -> &'static Instruments {
    static CELL: OnceLock<Instruments> = OnceLock::new();
    CELL.get_or_init(|| {
        let meter = opentelemetry::global::meter("vellum");
        Instruments {
            rows: meter
                .u64_counter(ROWS_COMPUTED_TOTAL)
                .with_description("Collection rows recomputed")
                .build(),
            compute: meter
                .f64_histogram(COMPUTE_SECONDS)
                .with_unit("s")
                .with_description("Collection compute batch latency")
                .build(),
            formula_errors: meter
                .u64_counter(FORMULA_ERRORS_TOTAL)
                .with_description("Formula cells that evaluated to an error")
                .build(),
            query: meter
                .f64_histogram(QUERY_SECONDS)
                .with_unit("s")
                .with_description("Collection view query latency")
                .build(),
            csv_rows: meter
                .u64_counter(CSV_ROWS_TOTAL)
                .with_description("CSV rows imported or exported")
                .build(),
        }
    })
}

pub(crate) fn computed(rows: usize, started: Instant) {
    let i = instruments();
    i.rows.add(u64::try_from(rows).unwrap_or(u64::MAX), &[]);
    i.compute.record(started.elapsed().as_secs_f64(), &[]);
}

pub(crate) fn formula_error() {
    instruments().formula_errors.add(1, &[]);
}

pub(crate) fn queried(kind: &'static str, started: Instant) {
    instruments().query.record(
        started.elapsed().as_secs_f64(),
        &[KeyValue::new("kind", kind)],
    );
}

pub(crate) fn csv(direction: &'static str, rows: usize) {
    instruments().csv_rows.add(
        u64::try_from(rows).unwrap_or(u64::MAX),
        &[KeyValue::new("direction", direction)],
    );
}
