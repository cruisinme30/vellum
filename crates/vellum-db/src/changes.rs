// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive's page change log (ADR 0038 D164).
//!
//! Triggers on `drive_nodes` append a page's ID to `drive_changes`
//! whenever it is added, removed, or one of its searchable fields changes
//! (place, path, title, size, modified time, trash state). That covers
//! every writer, including rescans and the file watcher, without the
//! drive knowing. Search reads the log from a cursor, so its cost follows
//! writes rather than the number of pages.
//!
//! Positions are `(txid, seq)`. On SQLite there is one writer, so `seq`
//! commits in order and `txid` is always 0. On Postgres `seq` can commit
//! out of order, so a reader only takes rows from transactions older than
//! every transaction still running (`pg_snapshot_xmin`), in `(txid, seq)`
//! order: no row can later appear behind the cursor, and writers share no
//! lock. A long-running transaction elsewhere in the database therefore
//! delays the log until it ends.

use std::time::Duration;

use uuid::Uuid;

use crate::cluster::now_ms;
use crate::error::Result;
use crate::index::{Index, IndexPool};
use crate::telemetry::traced;

/// A place in the log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// The writing transaction on Postgres; 0 on SQLite.
    pub txid: i64,
    /// The row's number.
    pub seq: i64,
}

impl Position {
    /// As the pair [`crate::cluster::Cursors`] stores.
    #[must_use]
    pub const fn pair(self) -> (i64, i64) {
        (self.txid, self.seq)
    }

    /// From the pair [`crate::cluster::Cursors`] stores.
    #[must_use]
    pub const fn from_pair((txid, seq): (i64, i64)) -> Self {
        Self { txid, seq }
    }
}

/// What [`DriveChanges::read`] found past a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Pages that changed, in log order; a page may appear more than once.
    pub pages: Vec<Uuid>,
    /// Where to read from next time.
    pub next: Position,
    /// The position was pruned past: changes were missed, and the reader
    /// must treat every page as changed. `next` is then the head as it was
    /// before this call returned.
    pub reset: bool,
}

/// The log in an index (migrated by `Index::open`).
#[derive(Debug, Clone)]
pub struct DriveChanges {
    index: Index,
}

type Row = (i64, i64, Uuid);

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

impl DriveChanges {
    /// The log in `index`.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    async fn horizon(&self) -> Result<Position> {
        let system = self.index.backend().otel_system();
        let sql = "SELECT horizon_txid, horizon_seq FROM drive_change_meta WHERE id = 1";
        let (txid, seq): (i64, i64) = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    system,
                    "changes.horizon",
                    sqlx::query_as(sql).fetch_one(pool),
                )
                .await?
            }
            IndexPool::Postgres(pool) => {
                traced(
                    system,
                    "changes.horizon",
                    sqlx::query_as(sql).fetch_one(pool),
                )
                .await?
            }
        };
        Ok(Position { txid, seq })
    }

    /// The position after everything committed so far: read from here to
    /// see only later changes.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn head(&self) -> Result<Position> {
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let seq: i64 = traced(
                    system,
                    "changes.head",
                    sqlx::query_scalar(
                        "SELECT MAX(COALESCE((SELECT MAX(seq) FROM drive_changes), 0), \
                         horizon_seq) FROM drive_change_meta WHERE id = 1",
                    )
                    .fetch_one(pool),
                )
                .await?;
                Ok(Position { txid: 0, seq })
            }
            IndexPool::Postgres(pool) => {
                let txid: i64 = traced(
                    system,
                    "changes.head",
                    sqlx::query_scalar(
                        "SELECT pg_snapshot_xmin(pg_current_snapshot())::text::bigint",
                    )
                    .fetch_one(pool),
                )
                .await?;
                Ok(Position { txid, seq: 0 })
            }
        }
    }

    /// Up to `limit` changes after `after`.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn read(&self, after: Position, limit: usize) -> Result<Batch> {
        if after < self.horizon().await? {
            return Ok(Batch {
                pages: Vec::new(),
                next: self.head().await?,
                reset: true,
            });
        }
        let limit_n = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        let system = self.index.backend().otel_system();
        let (rows, settled): (Vec<Row>, Position) = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let rows = traced(
                    system,
                    "changes.read",
                    sqlx::query_as(
                        "SELECT txid, seq, node_id FROM drive_changes WHERE seq > $1 \
                         ORDER BY seq LIMIT $2",
                    )
                    .bind(after.seq)
                    .bind(limit_n)
                    .fetch_all(pool),
                )
                .await?;
                (rows, after)
            }
            IndexPool::Postgres(pool) => {
                // Everything below xmin has committed or rolled back.
                let head = self.head().await?;
                let rows = traced(
                    system,
                    "changes.read",
                    sqlx::query_as(
                        "SELECT txid, seq, node_id FROM drive_changes \
                         WHERE (txid, seq) > ($1, $2) AND txid < $3 \
                         ORDER BY txid, seq LIMIT $4",
                    )
                    .bind(after.txid)
                    .bind(after.seq)
                    .bind(head.txid)
                    .bind(limit_n)
                    .fetch_all(pool),
                )
                .await?;
                (rows, head.max(after))
            }
        };
        let full = rows.len() >= limit.max(1);
        let last = rows.last().map(|&(txid, seq, _)| Position { txid, seq });
        let next = match last {
            // More may wait past the last row read.
            Some(last) if full => last,
            Some(last) => last.max(settled),
            None => settled,
        };
        Ok(Batch {
            pages: rows.into_iter().map(|(_, _, page)| page).collect(),
            next,
            reset: false,
        })
    }

    /// Deletes changes recorded more than `older_than` ago and moves the
    /// horizon past them. Returns how many went.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn prune(&self, older_than: Duration) -> Result<u64> {
        let before = now_ms().saturating_sub(ms(older_than));
        let system = self.index.backend().otel_system();
        let newest = "SELECT txid, seq FROM drive_changes WHERE at_ms < $1 \
                      ORDER BY txid DESC, seq DESC LIMIT 1";
        let raise = "UPDATE drive_change_meta SET horizon_txid = $1, horizon_seq = $2 \
                     WHERE id = 1 AND (horizon_txid < $1 OR (horizon_txid = $1 AND horizon_seq < $2))";
        let delete = "DELETE FROM drive_changes WHERE (txid, seq) <= ($1, $2)";
        macro_rules! run {
            ($pool:expr) => {{
                let mut tx = $pool.begin().await?;
                let top: Option<(i64, i64)> = traced(
                    system,
                    "changes.prune_top",
                    sqlx::query_as(newest).bind(before).fetch_optional(&mut *tx),
                )
                .await?;
                let Some((txid, seq)) = top else {
                    return Ok(0);
                };
                let _ = traced(
                    system,
                    "changes.horizon_raise",
                    sqlx::query(raise).bind(txid).bind(seq).execute(&mut *tx),
                )
                .await?;
                let gone = traced(
                    system,
                    "changes.prune",
                    sqlx::query(delete).bind(txid).bind(seq).execute(&mut *tx),
                )
                .await?
                .rows_affected();
                tx.commit().await?;
                Ok(gone)
            }};
        }
        match self.index.pool() {
            IndexPool::Sqlite(pool) => run!(pool),
            IndexPool::Postgres(pool) => run!(pool),
        }
    }
}
