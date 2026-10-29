// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! DB spans: every statement gets `db.system`, `db.statement`, `db.rows` and
//! `db.duration_ms`, plus a `vellum.db.duration_ms` histogram for RED dashboards.

use std::future::Future;
use std::time::Instant;

use tracing::{Instrument as _, field};

/// Something that knows how many rows it touched.
pub trait RowCount {
    /// Rows returned or affected.
    fn row_count(&self) -> u64;
}

impl RowCount for sqlx::sqlite::SqliteQueryResult {
    fn row_count(&self) -> u64 {
        self.rows_affected()
    }
}

impl RowCount for sqlx::postgres::PgQueryResult {
    fn row_count(&self) -> u64 {
        self.rows_affected()
    }
}

impl<T> RowCount for Vec<T> {
    fn row_count(&self) -> u64 {
        u64::try_from(self.len()).unwrap_or(u64::MAX)
    }
}

impl RowCount for () {
    fn row_count(&self) -> u64 {
        0
    }
}

/// Runs `fut` inside a `db.query` span and records rows and elapsed milliseconds.
///
/// `statement` is a short, stable name (never SQL with values in it), so spans
/// can be grouped and nothing sensitive reaches the trace.
pub async fn traced<T, E, F>(system: &'static str, statement: &'static str, fut: F) -> Result<T, E>
where
    T: RowCount,
    F: Future<Output = Result<T, E>>,
{
    let span = tracing::debug_span!(
        "db.query",
        db.system = system,
        db.statement = statement,
        db.rows = field::Empty,
        db.duration_ms = field::Empty,
        otel.status_code = field::Empty,
    );
    let start = Instant::now();
    let out = fut.instrument(span.clone()).await;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let _ = span.record("db.duration_ms", ms);
    tracing::debug!(
        histogram.vellum.db.duration_ms = ms,
        db.system = system,
        db.statement = statement
    );
    if let Ok(value) = &out {
        let _ = span.record("db.rows", value.row_count());
    } else {
        let _ = span.record("otel.status_code", "ERROR");
    }
    out
}
