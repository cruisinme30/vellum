// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rate limits that hold for the whole cluster (ADR 0040, D173).
//!
//! The gateway's own limiter keeps bursts in check on each replica. On top
//! of it, [`ClusterLimit`] counts each caller's requests in one-minute
//! windows and adds its counts to the index database every second, getting
//! back what every replica counted. A caller over the per-minute allowance
//! is refused on every replica, at most about a second late.
//!
//! Requests carrying a live-editing join signed by another replica are
//! exempt, since every editor that replica forwards shares its address.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::http::HeaderMap;
use vellum_api::gateway::{RateExempt, SharedLimit};
use vellum_collab::cluster::{FORWARD_HEADER, is_signed_forward};
use vellum_db::cluster::now_ms;
use vellum_db::rate::SharedCounts;

/// Variable holding the key replicas sign forwarded joins with.
pub const CLUSTER_KEY_VAR: &str = "VELLUM_CLUSTER_KEY";

/// Length of a counting window.
pub const WINDOW: Duration = Duration::from_mins(1);

/// How often counts are shared.
const SYNC_EVERY: Duration = Duration::from_secs(1);

/// Most callers tracked per window; past it, new callers are only held to
/// each replica's own limiter.
const MAX_CALLERS: usize = 100_000;

fn window_ms() -> i64 {
    i64::try_from(WINDOW.as_millis()).unwrap_or(i64::MAX)
}

#[derive(Debug, Default)]
struct Counts {
    /// Start of the current window, in milliseconds since the epoch.
    window: i64,
    /// Requests counted here and not yet shared.
    pending: HashMap<String, u64>,
    /// The cluster's totals at the last share.
    totals: HashMap<String, u64>,
}

impl Counts {
    /// Moves to the window holding `now`, forgetting the last one.
    fn roll(&mut self, now: i64) {
        let window = now - now.rem_euclid(window_ms());
        if window != self.window {
            self.window = window;
            self.pending.clear();
            self.totals.clear();
        }
    }
}

/// A per-minute allowance per caller, shared through the index database.
#[derive(Debug)]
pub struct ClusterLimit {
    per_window: u64,
    counts: Mutex<Counts>,
}

impl ClusterLimit {
    /// An allowance of `per_window` requests per caller per minute.
    #[must_use]
    pub fn new(per_window: u64) -> Self {
        Self {
            per_window: per_window.max(1),
            counts: Mutex::new(Counts::default()),
        }
    }

    /// Shares counts through `shared` every second until the server stops.
    pub fn spawn_sync(self: &Arc<Self>, shared: SharedCounts) {
        let me = Arc::clone(self);
        drop(tokio::spawn(async move {
            let mut tick = tokio::time::interval(SYNC_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let _ = tick.tick().await;
                me.sync(&shared).await;
            }
        }));
    }

    /// Adds the pending counts to the database and takes the totals back.
    pub async fn sync(&self, shared: &SharedCounts) {
        self.sync_at(shared, now_ms()).await;
    }

    async fn sync_at(&self, shared: &SharedCounts, now: i64) {
        let (window, pending) = {
            let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            counts.roll(now);
            (counts.window, std::mem::take(&mut counts.pending))
        };
        if pending.is_empty() {
            return;
        }
        let hits: Vec<(String, u64)> = pending.into_iter().collect();
        match shared.add(window, &hits).await {
            Ok(totals) => {
                let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
                if counts.window == window {
                    counts.totals.extend(totals);
                }
            }
            Err(error) => tracing::warn!(%error, "rate counts not shared; limits are per replica"),
        }
    }

    fn hit_at(&self, caller: &str, now: i64) -> Option<Duration> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.roll(now);
        let known = counts.totals.contains_key(caller) || counts.pending.contains_key(caller);
        if !known && counts.totals.len() + counts.pending.len() >= MAX_CALLERS {
            return None;
        }
        let pending = {
            let n = counts.pending.entry(caller.to_owned()).or_default();
            *n += 1;
            *n
        };
        let seen = counts.totals.get(caller).copied().unwrap_or_default() + pending;
        let window = counts.window;
        drop(counts);
        if seen <= self.per_window {
            return None;
        }
        let left = window + window_ms() - now;
        Some(Duration::from_millis(
            u64::try_from(left.max(1_000)).unwrap_or(1_000),
        ))
    }
}

impl SharedLimit for ClusterLimit {
    fn hit(&self, caller: &str) -> Option<Duration> {
        self.hit_at(caller, now_ms())
    }
}

/// Exempts live-editing joins forwarded by another replica, when the
/// cluster key is set.
#[must_use]
pub fn forwarded_joins(var: impl Fn(&str) -> Option<String>) -> Option<RateExempt> {
    let key = var(CLUSTER_KEY_VAR)?.into_bytes();
    if key.len() < 32 {
        tracing::warn!("{CLUSTER_KEY_VAR} is shorter than 32 bytes; ignoring it");
        return None;
    }
    Some(Arc::new(move |headers: &HeaderMap, path: &str| {
        headers
            .get(FORWARD_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|header| is_signed_forward(&key, header, path))
    }))
}

#[cfg(test)]
mod tests {
    use vellum_db::{Index, IndexConfig};

    use super::*;

    #[tokio::test]
    async fn the_allowance_holds_across_replicas() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None))
            .await
            .unwrap();
        let shared = SharedCounts::new(index);
        let (a, b) = (ClusterLimit::new(5), ClusterLimit::new(5));
        let now = now_ms();
        for _ in 0..3 {
            assert_eq!(a.hit_at("ip:1", now), None);
        }
        a.sync_at(&shared, now).await;
        // b has seen nothing yet, then learns a's three.
        assert_eq!(b.hit_at("ip:1", now), None);
        b.sync_at(&shared, now).await;
        assert_eq!(b.hit_at("ip:1", now), None);
        assert!(
            b.hit_at("ip:1", now).is_some(),
            "3 + 2 used, the 6th is refused"
        );
        // Other callers are unaffected.
        assert_eq!(b.hit_at("ip:2", now), None);
    }
}
