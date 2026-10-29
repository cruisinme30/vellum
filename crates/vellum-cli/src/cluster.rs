// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Background work done once per cluster, not once per replica (ADR 0038,
//! D163).
//!
//! Every replica beats in the shared heartbeat table. Work that only one
//! replica should do runs under a lease in the index database:
//!
//! - the startup rescan of the workspace files runs only when no other
//!   replica is alive (one that is has kept the index current), and only
//!   on the replica that takes the `drive.rescan` lease;
//! - pruning the change feed, the page change log, finished jobs and old
//!   rate counts runs on whichever replica holds `maintenance`;
//! - the Google Calendar sync runs on whichever holds `tasks.google_sync`.
//!
//! On a single server all of this simply runs there, as before.

use std::future::Future;
use std::time::Duration;

use vellum_db::Index;
use vellum_db::changes::DriveChanges;
use vellum_db::cluster::{Leases, Node, Nodes, singleton};
use vellum_db::feed::Feed;
use vellum_db::jobs::Queue;
use vellum_db::rate::SharedCounts;

/// Variable with the address other replicas reach this one at
/// (`host:port`), set from the pod IP in Kubernetes.
pub const ADDRESS_VAR: &str = "VELLUM_NODE_ADDRESS";

/// How often a replica records that it is alive.
const BEAT_EVERY: Duration = Duration::from_secs(5);

/// A replica that has not beaten for this long is gone.
pub const LIVE_WINDOW: Duration = Duration::from_secs(15);

/// How long a lease lasts without renewal; a stopped holder's work moves
/// to another replica within about this long.
const TTL: Duration = Duration::from_secs(30);

/// The rescan lease. A rescan of a large workspace takes minutes; the
/// holder renews every third of the TTL while it runs.
const RESCAN: &str = "drive.rescan";

/// How often maintenance runs, and what it keeps.
const MAINTAIN_EVERY: Duration = Duration::from_mins(5);
const KEEP_PAGE_CHANGES: Duration = Duration::from_hours(24);
const KEEP_FINISHED_JOBS: Duration = Duration::from_hours(24 * 7);

/// This replica in the heartbeat table.
#[derive(Debug, Clone)]
pub struct Replica {
    nodes: Nodes,
    leases: Leases,
    me: Node,
    beating: tokio::task::AbortHandle,
}

impl Replica {
    /// Joins the cluster as `me` (unique per process, see
    /// [`crate::feed::origin`]) and keeps beating until the server stops.
    /// Returns the replica and whether another replica was already alive.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn join(
        index: &Index,
        me: &str,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<(Self, bool), vellum_db::Error> {
        let nodes = Nodes::new(index.clone());
        let others = nodes
            .live(LIVE_WINDOW)
            .await?
            .iter()
            .any(|node| node.name != me);
        let me = Node {
            name: me.to_owned(),
            address: var(ADDRESS_VAR).unwrap_or_default(),
        };
        nodes.beat(&me).await?;
        let (beats, node) = (nodes.clone(), me.clone());
        let beating = tokio::spawn(async move {
            let mut tick = tokio::time::interval(BEAT_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let _ = tick.tick().await;
                if let Err(error) = beats.beat(&node).await {
                    tracing::warn!(%error, "cannot record this replica's heartbeat");
                }
            }
        })
        .abort_handle();
        Ok((
            Self {
                nodes,
                leases: Leases::new(index.clone()),
                me,
                beating,
            },
            others,
        ))
    }

    /// Runs `rescan` unless another replica was alive when this one
    /// joined, or another replica is rescanning right now.
    pub async fn rescan_once<F>(&self, others_alive: bool, rescan: F)
    where
        F: Future<Output = ()>,
    {
        if others_alive {
            tracing::info!("another replica is running; skipping the startup rescan");
            return;
        }
        match self.leases.hold(RESCAN, &self.me.name, TTL, rescan).await {
            Ok(Some(())) => {}
            Ok(None) => {
                tracing::info!("another replica is rescanning; skipping the startup rescan");
            }
            Err(error) => {
                tracing::warn!(%error, "cannot take the rescan lease; skipping the rescan");
            }
        }
    }

    /// Runs `tick()` every `period` on one replica of the cluster at a
    /// time, under lease `name`.
    pub fn every<F, Fut>(&self, name: &str, period: Duration, tick: F)
    where
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send,
    {
        drop(singleton(
            self.leases.clone(),
            name,
            &self.me.name,
            TTL,
            move || {
                let tick = tick.clone();
                async move {
                    let mut timer = tokio::time::interval(period);
                    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        let _ = timer.tick().await;
                        tick().await;
                    }
                }
            },
        ));
    }

    /// Prunes the change feed, the page change log and finished jobs,
    /// on one replica at a time.
    pub fn maintain(&self, index: &Index) {
        let (feed, changes, jobs, rates) = (
            Feed::new(index.clone()),
            DriveChanges::new(index.clone()),
            Queue::new(index.clone()),
            SharedCounts::new(index.clone()),
        );
        self.every("maintenance", MAINTAIN_EVERY, move || {
            let (feed, changes, jobs, rates) =
                (feed.clone(), changes.clone(), jobs.clone(), rates.clone());
            async move {
                let window = i64::try_from(crate::limits::WINDOW.as_millis()).unwrap_or(i64::MAX);
                let before = vellum_db::cluster::now_ms().saturating_sub(2 * window);
                if let Err(error) = rates.prune(before).await {
                    tracing::warn!(%error, "cannot prune old rate counts");
                }
                match feed.prune(crate::feed::KEEP).await {
                    Ok(gone) => tracing::debug!(gone, "pruned the change feed"),
                    Err(error) => tracing::warn!(%error, "cannot prune the change feed"),
                }
                match changes.prune(KEEP_PAGE_CHANGES).await {
                    Ok(gone) => tracing::debug!(gone, "pruned the page change log"),
                    Err(error) => tracing::warn!(%error, "cannot prune the page change log"),
                }
                match jobs.prune(KEEP_FINISHED_JOBS).await {
                    Ok(gone) => tracing::debug!(gone, "pruned finished jobs"),
                    Err(error) => tracing::warn!(%error, "cannot prune finished jobs"),
                }
            }
        });
    }

    /// Removes this replica from the heartbeat table, on a clean shutdown.
    pub async fn leave(&self) {
        self.beating.abort();
        if let Err(error) = self.nodes.leave(&self.me).await {
            tracing::warn!(%error, "cannot remove this replica from the heartbeat table");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use vellum_db::IndexConfig;

    use super::*;

    #[tokio::test]
    async fn only_the_first_replica_rescans() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None))
            .await
            .unwrap();
        let rescans = Arc::new(AtomicU32::new(0));
        let rescan = |count: Arc<AtomicU32>| async move {
            let _ = count.fetch_add(1, Ordering::SeqCst);
        };

        let (a, others) = Replica::join(&index, "pod-a:1:1", |_| None).await.unwrap();
        assert!(!others);
        a.rescan_once(others, rescan(Arc::clone(&rescans))).await;
        let (b, others) = Replica::join(&index, "pod-b:1:1", |_| None).await.unwrap();
        assert!(others);
        b.rescan_once(others, rescan(Arc::clone(&rescans))).await;
        assert_eq!(rescans.load(Ordering::SeqCst), 1);

        // Once both have left, the next one to start rescans again.
        a.leave().await;
        b.leave().await;
        let (c, others) = Replica::join(&index, "pod-c:1:1", |_| None).await.unwrap();
        assert!(!others);
        c.rescan_once(others, rescan(Arc::clone(&rescans))).await;
        assert_eq!(rescans.load(Ordering::SeqCst), 2);
    }
}
