// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Export and import metrics, through the global OpenTelemetry meter.
//!
//! Spans come from `tracing::instrument` on each backend and importer;
//! these series add the RED view per format.

use std::sync::OnceLock;
use std::time::Instant;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};

/// Counter of exports, labelled `format` (`pdf`, `pdf_screen`, `docx`,
/// `gdocs`) and `outcome` (`ok` or an [`crate::ExportError::kind`]).
pub const EXPORTS_TOTAL: &str = "vellum_export_total";
/// Histogram of export latency in seconds, labelled `format`.
pub const EXPORT_SECONDS: &str = "vellum_export_duration_seconds";
/// Counter of exported bytes, labelled `format`.
pub const EXPORT_BYTES: &str = "vellum_export_bytes_total";
/// Counter of imports, labelled `source` and `outcome`.
pub const IMPORTS_TOTAL: &str = "vellum_import_total";
/// Counter of imported documents, labelled `source`.
pub const IMPORT_DOCUMENTS: &str = "vellum_import_documents_total";
/// Histogram of import latency in seconds, labelled `source`.
pub const IMPORT_SECONDS: &str = "vellum_import_duration_seconds";

struct Instruments {
    exports: Counter<u64>,
    export_seconds: Histogram<f64>,
    export_bytes: Counter<u64>,
    imports: Counter<u64>,
    import_documents: Counter<u64>,
    import_seconds: Histogram<f64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter = opentelemetry::global::meter("vellum");
        Instruments {
            exports: meter
                .u64_counter(EXPORTS_TOTAL)
                .with_description("Exports by format and outcome")
                .build(),
            export_seconds: meter
                .f64_histogram(EXPORT_SECONDS)
                .with_unit("s")
                .with_description("Export latency")
                .build(),
            export_bytes: meter
                .u64_counter(EXPORT_BYTES)
                .with_unit("By")
                .with_description("Bytes of exported files")
                .build(),
            imports: meter
                .u64_counter(IMPORTS_TOTAL)
                .with_description("Imports by source and outcome")
                .build(),
            import_documents: meter
                .u64_counter(IMPORT_DOCUMENTS)
                .with_description("Documents created by imports")
                .build(),
            import_seconds: meter
                .f64_histogram(IMPORT_SECONDS)
                .with_unit("s")
                .with_description("Import latency")
                .build(),
        }
    })
}

/// Records one finished export: its outcome, latency and size.
pub fn export_done(
    format: &'static str,
    started: Instant,
    result: Result<usize, &crate::ExportError>,
) {
    let m = instruments();
    let fmt = KeyValue::new("format", format);
    let outcome = result.as_ref().map_or_else(|e| e.kind(), |_| "ok");
    m.exports
        .add(1, &[fmt.clone(), KeyValue::new("outcome", outcome)]);
    m.export_seconds
        .record(started.elapsed().as_secs_f64(), std::slice::from_ref(&fmt));
    if let Ok(bytes) = result {
        m.export_bytes.add(bytes as u64, &[fmt]);
    }
}

/// Records one finished import: its outcome, latency and document count.
pub fn import_done(
    source: &'static str,
    started: Instant,
    result: Result<usize, &crate::ImportError>,
) {
    let m = instruments();
    let src = KeyValue::new("source", source);
    let outcome = result.as_ref().map_or_else(|e| e.kind(), |_| "ok");
    m.imports
        .add(1, &[src.clone(), KeyValue::new("outcome", outcome)]);
    m.import_seconds
        .record(started.elapsed().as_secs_f64(), std::slice::from_ref(&src));
    if let Ok(documents) = result {
        m.import_documents.add(documents as u64, &[src]);
    }
}
