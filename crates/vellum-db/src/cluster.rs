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
//! [`Leases::hold`] and [`singleton`] run work under a lease, and
//! [`Cursors`] keeps such work's position for the next holder (ADR 0038).
//!
//! Both use the replicas' clocks (milliseconds since the Unix epoch), so
//! clocks must agree to within the windows chosen, which NTP gives. See
//! ADR 0037.

use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::Instrument as _;

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
                        "INSERT INTO cluster_nodes (name, address, \
                         heartbeat_ms) VALUES (?1, ?2, ?3) ON CONFLICT (name) \
                         DO UPDATE SET address = excluded.address, \
                         heartbeat_ms = excluded.heartbeat_ms",
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
                        "INSERT INTO cluster_nodes (name, address, \
                         heartbeat_ms) VALUES ($1, $2, $3) ON CONFLICT (name) \
                         DO UPDATE SET address = excluded.address, \
                         heartbeat_ms = excluded.heartbeat_ms",
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
    pub async fn live_at(
        &self,
        window: Duration,
        at_ms: i64,
    ) -> Result<Vec<Node>> {
        let since = at_ms.saturating_sub(ms(window));
        let system = self.index.backend().otel_system();
        let rows: Vec<(String, String)> = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    system,
                    "cluster.live",
                    sqlx::query_as(
                        "SELECT name, address FROM cluster_nodes WHERE \
                            heartbeat_ms >= ?1 ORDER BY name",
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
                        "SELECT name, address FROM cluster_nodes WHERE \
                            heartbeat_ms >= $1 ORDER BY name",
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
    pub async fn acquire(
        &self,
        name: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool> {
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
                    "INSERT INTO cluster_leases (name, holder, expires_ms) \
                     VALUES (?1, ?2, ?3) ON CONFLICT (name) DO UPDATE SET \
                     holder = excluded.holder, expires_ms = \
                     excluded.expires_ms WHERE cluster_leases.holder = \
                     excluded.holder OR cluster_leases.expires_ms < ?4",
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
                    "INSERT INTO cluster_leases (name, holder, expires_ms) \
                     VALUES ($1, $2, $3) ON CONFLICT (name) DO UPDATE SET \
                     holder = excluded.holder, expires_ms = \
                     excluded.expires_ms WHERE cluster_leases.holder = \
                     excluded.holder OR cluster_leases.expires_ms < $4",
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
                    sqlx::query(
                        "DELETE FROM cluster_leases WHERE name = ?1 \
                        AND holder = ?2",
                    )
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
                    sqlx::query(
                        "DELETE FROM cluster_leases WHERE name = $1 \
                        AND holder = $2",
                    )
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

impl Leases {
    /// Runs `work` while `holder` holds lease `name` (D163). Takes the
    /// lease, renews it every third of `ttl`, and releases it when `work`
    /// ends. Returns `None` without running `work` when another holder
    /// has the lease, and drops `work` (cancelling it) as soon as a
    /// renewal finds the lease lost.
    ///
    /// A holder cut off from the database keeps running until its next
    /// renewal fails, so for up to a third of `ttl` two holders may
    /// overlap: work run this way must be safe to repeat.
    ///
    /// # Errors
    /// When the database is unreachable while taking the lease.
    pub async fn hold<F, T>(
        &self,
        name: &str,
        holder: &str,
        ttl: Duration,
        work: F,
    ) -> Result<Option<T>>
    where
        F: Future<Output = T>,
    {
        if !self.acquire(name, holder, ttl).await? {
            return Ok(None);
        }
        let mut renew = tokio::time::interval(ttl / 3);
        renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = renew.tick().await;
        tokio::pin!(work);
        loop {
            tokio::select! {
                out = &mut work => {
                    if let Err(error) = self.release(name, holder).await {
                        tracing::warn!(%error, lease = name, "cannot release \
                            the lease");
                    }
                    return Ok(Some(out));
                }
                _ = renew.tick() => match self.acquire(
                    name,
                    holder,
                    ttl
                ).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(lease = name, "lost the lease; stopping \
                            its work");
                        return Ok(None);
                    }
                    // Keep going: the lease still runs until it expires,
                    // and the next renewal may reach the database.
                    Err(
                        error
                    ) => tracing::warn!(
                        %error,
                        lease = name,
                        "cannot renew the lease"
                    ),
                },
            }
        }
    }
}

/// Keeps one copy of some background work running in the whole cluster
/// (D163).
///
/// Every replica calls this with the same `name`, and whichever takes
/// lease `name` runs `make()`'s future while it holds it. When the
/// holder stops or loses the lease, another replica takes over within
/// about `ttl`. When the work ends, the lease is released and the work is
/// started again by whoever takes it next.
///
/// The task runs until the handle is aborted.
#[must_use]
pub fn singleton<F, Fut>(
    leases: Leases,
    name: &str,
    holder: &str,
    ttl: Duration,
    mut make: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let (name, holder) = (name.to_owned(), holder.to_owned());
    tokio::spawn(async move {
        loop {
            let span = tracing::info_span!(
                "singleton",
                lease = %name,
                holder = %holder
            );
            match leases
                .hold(&name, &holder, ttl, make().instrument(span))
                .await
            {
                Ok(Some(()) | None) => {}
                Err(error) => {
                    tracing::warn!(%error, lease = %name, "cannot take the \
                        lease");
                }
            }
            tokio::time::sleep(ttl / 3).await;
        }
    })
}

/// A reader's position, kept in the database so the next replica to run
/// the reader carries on where the last one stopped.
#[derive(Debug, Clone)]
pub struct Cursors {
    index: Index,
}

impl Cursors {
    /// The table in `index` (migrated by `Index::open`).
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// The saved position `(major, minor)` of `name`, if any.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn get(&self, name: &str) -> Result<Option<(i64, i64)>> {
        let system = self.index.backend().otel_system();
        let sql = "SELECT major, minor FROM cluster_cursors WHERE name = $1";
        Ok(match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let query = sqlx::query_as(sql).bind(name);
                traced(system, "cluster.cursor", query.fetch_optional(pool))
                    .await?
            }
            IndexPool::Postgres(pool) => {
                let query = sqlx::query_as(sql).bind(name);
                traced(system, "cluster.cursor", query.fetch_optional(pool))
                    .await?
            }
        })
    }

    /// Saves `name` at `(major, minor)`, but only while `holder` holds
    /// lease `lease`, so a replica that lost its lease cannot move the
    /// cursor under the new holder. Returns whether it was saved.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn set(
        &self,
        name: &str,
        (major, minor): (i64, i64),
        lease: &str,
        holder: &str,
    ) -> Result<bool> {
        let system = self.index.backend().otel_system();
        let now = now_ms();
        let sql = "INSERT INTO cluster_cursors (name, major, minor, \
                   updated_ms) SELECT $1, $2, $3, $4 WHERE EXISTS (SELECT 1 \
                   FROM cluster_leases WHERE name = $5 AND holder = $6 AND \
                   expires_ms >= $4) ON CONFLICT (name) DO UPDATE SET major = \
                   excluded.major, minor = excluded.minor, updated_ms = \
                   excluded.updated_ms";
        let affected = match self.index.pool() {
            IndexPool::Sqlite(pool) => traced(
                system,
                "cluster.cursor_set",
                sqlx::query(sql)
                    .bind(name)
                    .bind(major)
                    .bind(minor)
                    .bind(now)
                    .bind(lease)
                    .bind(holder)
                    .execute(pool),
            )
            .await?
            .rows_affected(),
            IndexPool::Postgres(pool) => traced(
                system,
                "cluster.cursor_set",
                sqlx::query(sql)
                    .bind(name)
                    .bind(major)
                    .bind(minor)
                    .bind(now)
                    .bind(lease)
                    .bind(holder)
                    .execute(pool),
            )
            .await?
            .rows_affected(),
        };
        Ok(affected == 1)
    }
}
