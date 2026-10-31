// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Search spans and metrics, exported through `vellum-kernel::telemetry`.
//!
//! Every backend call runs in a `search.backend` span carrying the engine,
//! the operation, the row count and the elapsed milliseconds, and feeds the
//! `vellum.search.duration_ms` histogram (RED per engine and operation).
//! Spans never carry query text or document content.

use std::future::Future;
use std::time::Instant;

use tracing::{Instrument as _, field};

/// Something that knows how many rows it holds.
pub trait RowCount {
    /// Rows returned or written.
    fn row_count(&self) -> u64;
}

impl<T> RowCount for Vec<T> {
    fn row_count(&self) -> u64 {
        u64::try_from(self.len()).unwrap_or(u64::MAX)
    }
}

impl<T> RowCount for Option<T> {
    fn row_count(&self) -> u64 {
        u64::from(self.is_some())
    }
}

impl RowCount for () {
    fn row_count(&self) -> u64 {
        0
    }
}

/// Runs `fut` in a `search.backend` span and records rows and milliseconds.
///
/// # Errors
/// Whatever `fut` returns.
pub async fn traced<T, E, F>(
    engine: &'static str,
    operation: &'static str,
    fut: F,
) -> Result<T, E>
where
    T: RowCount,
    F: Future<Output = Result<T, E>>,
{
    let span = tracing::debug_span!(
        "search.backend",
        search.engine = engine,
        search.operation = operation,
        search.rows = field::Empty,
        search.duration_ms = field::Empty,
        otel.status_code = field::Empty,
    );
    let start = Instant::now();
    let out = fut.instrument(span.clone()).await;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let _ = span.record("search.duration_ms", ms);
    tracing::debug!(
        histogram.vellum.search.duration_ms = ms,
        search.engine = engine,
        search.operation = operation
    );
    if let Ok(value) = &out {
        let _ = span.record("search.rows", value.row_count());
    } else {
        {
            let _ = span.record("otel.status_code", "ERROR");
            tracing::debug!(
                counter.vellum.search.errors = 1_u64,
                search.engine = engine,
                search.operation = operation
            );
        }
    }
    out
}
