// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rate-limit counts shared by the replicas of one deployment (ADR 0040,
//! D173).
//!
//! Each replica counts requests per caller in memory and adds its counts
//! to `rate_counts` about once a second, getting back the cluster's total
//! for each caller in the current window. A replica therefore refuses a
//! caller once the whole cluster has used up the allowance, at most about
//! a second late.

use sqlx::QueryBuilder;

use crate::error::Result;
use crate::index::{Index, IndexPool};
use crate::telemetry::traced;

/// Rows written per statement.
const CHUNK: usize = 500;

/// The counts table in an index (migrated by `Index::open`).
#[derive(Debug, Clone)]
pub struct SharedCounts {
    index: Index,
}

impl SharedCounts {
    /// The table in `index`.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// Adds `hits` (caller, requests) to window `window_ms` and returns
    /// each caller's total for the window across every replica.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn add(&self, window_ms: i64, hits: &[(String, u64)]) -> Result<Vec<(String, u64)>> {
        let mut totals = Vec::with_capacity(hits.len());
        let system = self.index.backend().otel_system();
        for chunk in hits.chunks(CHUNK) {
            let rows: Vec<(String, i64)> = match self.index.pool() {
                IndexPool::Sqlite(pool) => {
                    let mut q =
                        QueryBuilder::new("INSERT INTO rate_counts (caller, window_ms, hits) ");
                    let _ = q.push_values(chunk, |mut row, (caller, n)| {
                        let _ = row
                            .push_bind(caller)
                            .push_bind(window_ms)
                            .push_bind(i64::try_from(*n).unwrap_or(i64::MAX));
                    });
                    let _ = q.push(
                        " ON CONFLICT (caller, window_ms) DO UPDATE \
                         SET hits = rate_counts.hits + excluded.hits RETURNING caller, hits",
                    );
                    traced(system, "rate.add", q.build_query_as().fetch_all(pool)).await?
                }
                IndexPool::Postgres(pool) => {
                    let mut q =
                        QueryBuilder::new("INSERT INTO rate_counts (caller, window_ms, hits) ");
                    let _ = q.push_values(chunk, |mut row, (caller, n)| {
                        let _ = row
                            .push_bind(caller)
                            .push_bind(window_ms)
                            .push_bind(i64::try_from(*n).unwrap_or(i64::MAX));
                    });
                    let _ = q.push(
                        " ON CONFLICT (caller, window_ms) DO UPDATE \
                         SET hits = rate_counts.hits + excluded.hits RETURNING caller, hits",
                    );
                    traced(system, "rate.add", q.build_query_as().fetch_all(pool)).await?
                }
            };
            totals.extend(
                rows.into_iter()
                    .map(|(caller, n)| (caller, u64::try_from(n).unwrap_or_default())),
            );
        }
        Ok(totals)
    }

    /// Deletes windows that started before `before_ms`.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn prune(&self, before_ms: i64) -> Result<u64> {
        let system = self.index.backend().otel_system();
        let sql = "DELETE FROM rate_counts WHERE window_ms < $1";
        Ok(match self.index.pool() {
            IndexPool::Sqlite(pool) => traced(
                system,
                "rate.prune",
                sqlx::query(sql).bind(before_ms).execute(pool),
            )
            .await?
            .rows_affected(),
            IndexPool::Postgres(pool) => traced(
                system,
                "rate.prune",
                sqlx::query(sql).bind(before_ms).execute(pool),
            )
            .await?
            .rows_affected(),
        })
    }
}
