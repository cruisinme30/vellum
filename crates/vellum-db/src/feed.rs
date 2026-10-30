// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The change feed the replicas of one deployment share.
//!
//! A replica that changes something other replicas may hold in memory (a
//! file, a space) appends a [`Change`]. Every replica follows the feed from
//! a cursor and drops what it cached about each subject. Positions commit
//! in order, because each append first bumps the single `cluster_feed`
//! row, so a reader that saw position `n` has seen everything before it.
//!
//! The feed only says *that* something changed, never the new content:
//! readers reload from the source of truth. Old positions are pruned; a
//! reader whose cursor falls behind the pruned horizon, or past the head
//! of a rebuilt index, is told to reset and reload everything. See ADR 0038.

use crate::error::Result;
use crate::index::{Index, IndexPool};
use crate::telemetry::traced;
use crate::tx::BeginWrite as _;

/// Topic of a changed file; the subject is its path.
pub const PATH: &str = "path";
/// Topic of a space that was created or changed; the subject is its root.
/// Readers reload the whole space list.
pub const SPACES: &str = "spaces";

/// One entry of the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Position in the feed, from 1.
    pub seq: i64,
    /// What kind of thing changed, such as `"path"` or `"spaces"`.
    pub topic: String,
    /// Which one (a file path), or empty when the topic is the whole thing.
    pub subject: String,
    /// The replica that made the change, so it can skip its own.
    pub origin: String,
}

/// What [`Feed::read`] found past a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Changes in order, at most the limit asked for.
    pub changes: Vec<Change>,
    /// The cursor to read from next time.
    pub cursor: i64,
    /// The cursor was pruned past or is ahead of the feed (the index was
    /// rebuilt): the reader missed changes and must reload everything.
    pub reset: bool,
}

/// The feed table in an index (migrated by `Index::open`).
#[derive(Debug, Clone)]
pub struct Feed {
    index: Index,
}

fn len_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Feed {
    /// The feed in `index`.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// Appends one change per `(topic, subject)` from `origin`, in one
    /// transaction, and returns the new head. Nothing is written for an
    /// empty list.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn publish(
        &self,
        origin: &str,
        items: &[(&str, &str)],
    ) -> Result<i64> {
        if items.is_empty() {
            return self.head().await;
        }
        let n = len_i64(items.len());
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let mut tx = pool.begin_write().await?;
                let (head,): (i64,) = traced(
                    system,
                    "feed.bump",
                    sqlx::query_as(
                        "UPDATE cluster_feed SET head = head + ?1 WHERE id = 1 \
                            RETURNING head",
                    )
                    .bind(n)
                    .fetch_one(&mut *tx),
                )
                .await?;
                for (i, (topic, subject)) in items.iter().enumerate() {
                    let _ = traced(
                        system,
                        "feed.append",
                        sqlx::query(
                            "INSERT INTO cluster_changes (seq, topic, subject, \
                                origin) VALUES (?1, ?2, ?3, ?4)",
                        )
                        .bind(head - n + 1 + len_i64(i))
                        .bind(topic)
                        .bind(subject)
                        .bind(origin)
                        .execute(&mut *tx),
                    )
                    .await?;
                }
                tx.commit().await?;
                Ok(head)
            }
            IndexPool::Postgres(pool) => {
                let mut tx = pool.begin_write().await?;
                let (head,): (i64,) = traced(
                    system,
                    "feed.bump",
                    sqlx::query_as(
                        "UPDATE cluster_feed SET head = head + $1 WHERE id = 1 \
                            RETURNING head",
                    )
                    .bind(n)
                    .fetch_one(&mut *tx),
                )
                .await?;
                for (i, (topic, subject)) in items.iter().enumerate() {
                    let _ = traced(
                        system,
                        "feed.append",
                        sqlx::query(
                            "INSERT INTO cluster_changes (seq, topic, subject, \
                                origin) VALUES ($1, $2, $3, $4)",
                        )
                        .bind(head - n + 1 + len_i64(i))
                        .bind(topic)
                        .bind(subject)
                        .bind(origin)
                        .execute(&mut *tx),
                    )
                    .await?;
                }
                tx.commit().await?;
                Ok(head)
            }
        }
    }

    /// The newest position, where a new reader starts.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn head(&self) -> Result<i64> {
        Ok(self.bounds().await?.0)
    }

    async fn bounds(&self) -> Result<(i64, i64)> {
        const SQL: &str = "SELECT head, horizon FROM cluster_feed WHERE id = 1";
        let system = self.index.backend().otel_system();
        Ok(match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    system,
                    "feed.bounds",
                    sqlx::query_as(SQL).fetch_one(pool),
                )
                .await?
            }
            IndexPool::Postgres(pool) => {
                traced(
                    system,
                    "feed.bounds",
                    sqlx::query_as(SQL).fetch_one(pool),
                )
                .await?
            }
        })
    }

    /// Up to `limit` changes after `cursor`.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn read(&self, cursor: i64, limit: usize) -> Result<Page> {
        let (head, horizon) = self.bounds().await?;
        if cursor < horizon || cursor > head {
            return Ok(Page {
                changes: Vec::new(),
                cursor: head,
                reset: true,
            });
        }
        let limit = len_i64(limit.max(1));
        let system = self.index.backend().otel_system();
        let rows: Vec<(i64, String, String, String)> = match self.index.pool() {
            IndexPool::Sqlite(pool) => traced(
                system,
                "feed.read",
                sqlx::query_as(
                    "SELECT seq, topic, subject, origin FROM cluster_changes \
                         WHERE seq > ?1 ORDER BY seq LIMIT ?2",
                )
                .bind(cursor)
                .bind(limit)
                .fetch_all(pool),
            )
            .await?,
            IndexPool::Postgres(pool) => traced(
                system,
                "feed.read",
                sqlx::query_as(
                    "SELECT seq, topic, subject, origin FROM cluster_changes \
                         WHERE seq > $1 ORDER BY seq LIMIT $2",
                )
                .bind(cursor)
                .bind(limit)
                .fetch_all(pool),
            )
            .await?,
        };
        // Pruned between the two reads: the first row is not the next one,
        // or the rows up to the head are gone.
        let pruned = match rows.first() {
            Some((seq, ..)) => *seq != cursor + 1,
            None => head > cursor,
        };
        if pruned {
            return Ok(Page {
                changes: Vec::new(),
                cursor: head.max(rows.last().map_or(head, |r| r.0)),
                reset: true,
            });
        }
        let cursor = rows.last().map_or(cursor, |r| r.0);
        Ok(Page {
            changes: rows
                .into_iter()
                .map(|(seq, topic, subject, origin)| Change {
                    seq,
                    topic,
                    subject,
                    origin,
                })
                .collect(),
            cursor,
            reset: false,
        })
    }

    /// Drops all but the newest `keep` positions and returns how many rows
    /// went.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn prune(&self, keep: u64) -> Result<u64> {
        let keep = i64::try_from(keep).unwrap_or(i64::MAX);
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let mut tx = pool.begin_write().await?;
                let (horizon,): (i64,) = traced(
                    system,
                    "feed.horizon",
                    sqlx::query_as(
                        "UPDATE cluster_feed SET horizon = max(horizon, head - \
                            ?1) WHERE id = 1 RETURNING horizon",
                    )
                    .bind(keep)
                    .fetch_one(&mut *tx),
                )
                .await?;
                let gone = traced(
                    system,
                    "feed.prune",
                    sqlx::query("DELETE FROM cluster_changes WHERE seq <= ?1")
                        .bind(horizon)
                        .execute(&mut *tx),
                )
                .await?
                .rows_affected();
                tx.commit().await?;
                Ok(gone)
            }
            IndexPool::Postgres(pool) => {
                let mut tx = pool.begin_write().await?;
                let (horizon,): (i64,) = traced(
                    system,
                    "feed.horizon",
                    sqlx::query_as(
                        "UPDATE cluster_feed SET horizon = GREATEST(horizon, \
                            head - $1) WHERE id = 1 RETURNING horizon",
                    )
                    .bind(keep)
                    .fetch_one(&mut *tx),
                )
                .await?;
                let gone = traced(
                    system,
                    "feed.prune",
                    sqlx::query("DELETE FROM cluster_changes WHERE seq <= $1")
                        .bind(horizon)
                        .execute(&mut *tx),
                )
                .await?
                .rows_affected();
                tx.commit().await?;
                Ok(gone)
            }
        }
    }
}
