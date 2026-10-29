// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The replicas of one deployment, and work only one of them does at a time.
//!
//! [`Nodes`] is a heartbeat table: each replica refreshes its row every few
//! seconds, and [`Nodes::live`] lists the rows newer than a window. Collab
//! uses it to agree which replica owns an editing room.
//!
//! [`Leases`] names singleton work (a sweep, a scheduler). The holder
//! renews before the lease expires; anyone may take it once it has.
//!
//! Both use the replicas' clocks (milliseconds since the Unix epoch), so
//! clocks must agree to within the windows chosen, which NTP gives. See
//! ADR 0037.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Result;
use crate::index::{Index, IndexPool};
use crate::telemetry::traced;

/// A replica: a stable name and the address other replicas reach it at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Node {
    /// Unique and stable for the replica's life (a pod name).
    pub name: String,
    /// `host:port` other replicas connect to.
    pub address: String,
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// The heartbeat table.
#[derive(Debug, Clone)]
pub struct Nodes {
    index: Index,
}

impl Nodes {
    /// The table in `index` (migrated by `Index::open`).
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// Records that `node` is alive now.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn beat(&self, node: &Node) -> Result<()> {
        self.beat_at(node, now_ms()).await
    }

    /// [`Self::beat`] at a given time, for tests.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn beat_at(&self, node: &Node, at_ms: i64) -> Result<()> {
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let _ = traced(
                    system,
                    "cluster.beat",
                    sqlx::query(
                        "INSERT INTO cluster_nodes (name, address, heartbeat_ms) VALUES (?1, ?2, ?3) \
                         ON CONFLICT (name) DO UPDATE SET address = excluded.address, heartbeat_ms = excluded.heartbeat_ms",
                    )
                    .bind(&node.name)
                    .bind(&node.address)
                    .bind(at_ms)
                    .execute(pool),
                )
                .await?;
            }
            IndexPool::Postgres(pool) => {
                let _ = traced(
                    system,
                    "cluster.beat",
                    sqlx::query(
                        "INSERT INTO cluster_nodes (name, address, heartbeat_ms) VALUES ($1, $2, $3) \
                         ON CONFLICT (name) DO UPDATE SET address = excluded.address, heartbeat_ms = excluded.heartbeat_ms",
                    )
                    .bind(&node.name)
                    .bind(&node.address)
                    .bind(at_ms)
                    .execute(pool),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Removes `node` at once, on a clean shutdown.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn leave(&self, node: &Node) -> Result<()> {
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let _ = traced(
                    system,
                    "cluster.leave",
                    sqlx::query("DELETE FROM cluster_nodes WHERE name = ?1")
                        .bind(&node.name)
                        .execute(pool),
                )
                .await?;
            }
            IndexPool::Postgres(pool) => {
                let _ = traced(
                    system,
                    "cluster.leave",
                    sqlx::query("DELETE FROM cluster_nodes WHERE name = $1")
                        .bind(&node.name)
                        .execute(pool),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The replicas that beat within `window`, by name.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn live(&self, window: Duration) -> Result<Vec<Node>> {
        self.live_at(window, now_ms()).await
    }

    /// [`Self::live`] at a given time, for tests.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn live_at(&self, window: Duration, at_ms: i64) -> Result<Vec<Node>> {
        let since = at_ms.saturating_sub(ms(window));
        let system = self.index.backend().otel_system();
        let rows: Vec<(String, String)> = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    system,
                    "cluster.live",
                    sqlx::query_as(
                        "SELECT name, address FROM cluster_nodes WHERE heartbeat_ms >= ?1 ORDER BY name",
                    )
                    .bind(since)
                    .fetch_all(pool),
                )
                .await?
            }
            IndexPool::Postgres(pool) => {
                traced(
                    system,
                    "cluster.live",
                    sqlx::query_as(
                        "SELECT name, address FROM cluster_nodes WHERE heartbeat_ms >= $1 ORDER BY name",
                    )
                    .bind(since)
                    .fetch_all(pool),
                )
                .await?
            }
        };
        Ok(rows
            .into_iter()
            .map(|(name, address)| Node { name, address })
            .collect())
    }
}

/// The lease table.
#[derive(Debug, Clone)]
pub struct Leases {
    index: Index,
}

impl Leases {
    /// The table in `index` (migrated by `Index::open`).
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// Takes or renews lease `name` for `holder` until `ttl` from now.
    /// Returns `false` while another holder's lease runs.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn acquire(&self, name: &str, holder: &str, ttl: Duration) -> Result<bool> {
        self.acquire_at(name, holder, ttl, now_ms()).await
    }

    /// [`Self::acquire`] at a given time, for tests.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn acquire_at(
        &self,
        name: &str,
        holder: &str,
        ttl: Duration,
        at_ms: i64,
    ) -> Result<bool> {
        let until = at_ms.saturating_add(ms(ttl));
        let system = self.index.backend().otel_system();
        let affected = match self.index.pool() {
            IndexPool::Sqlite(pool) => traced(
                system,
                "cluster.lease",
                sqlx::query(
                    "INSERT INTO cluster_leases (name, holder, expires_ms) VALUES (?1, ?2, ?3) \
                     ON CONFLICT (name) DO UPDATE SET holder = excluded.holder, expires_ms = excluded.expires_ms \
                     WHERE cluster_leases.holder = excluded.holder OR cluster_leases.expires_ms < ?4",
                )
                .bind(name)
                .bind(holder)
                .bind(until)
                .bind(at_ms)
                .execute(pool),
            )
            .await?
            .rows_affected(),
            IndexPool::Postgres(pool) => traced(
                system,
                "cluster.lease",
                sqlx::query(
                    "INSERT INTO cluster_leases (name, holder, expires_ms) VALUES ($1, $2, $3) \
                     ON CONFLICT (name) DO UPDATE SET holder = excluded.holder, expires_ms = excluded.expires_ms \
                     WHERE cluster_leases.holder = excluded.holder OR cluster_leases.expires_ms < $4",
                )
                .bind(name)
                .bind(holder)
                .bind(until)
                .bind(at_ms)
                .execute(pool),
            )
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }

    /// Gives lease `name` up, if `holder` holds it.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn release(&self, name: &str, holder: &str) -> Result<()> {
        let system = self.index.backend().otel_system();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let _ = traced(
                    system,
                    "cluster.release",
                    sqlx::query("DELETE FROM cluster_leases WHERE name = ?1 AND holder = ?2")
                        .bind(name)
                        .bind(holder)
                        .execute(pool),
                )
                .await?;
            }
            IndexPool::Postgres(pool) => {
                let _ = traced(
                    system,
                    "cluster.release",
                    sqlx::query("DELETE FROM cluster_leases WHERE name = $1 AND holder = $2")
                        .bind(name)
                        .bind(holder)
                        .execute(pool),
                )
                .await?;
            }
        }
        Ok(())
    }
}
