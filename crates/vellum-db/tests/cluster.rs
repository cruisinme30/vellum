// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Replica heartbeats and leases (ADR 0037) on the SQLite backend.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use std::time::Duration;

use vellum_db::cluster::{Leases, Node, Nodes};
use vellum_db::{Index, IndexConfig};

async fn index(dir: &tempfile::TempDir) -> Index {
    Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None))
        .await
        .unwrap()
}

fn node(name: &str) -> Node {
    Node {
        name: name.to_owned(),
        address: format!("{name}:7700"),
    }
}

#[tokio::test]
async fn live_nodes_are_the_ones_that_beat_recently() {
    let dir = tempfile::tempdir().unwrap();
    let nodes = Nodes::new(index(&dir).await);
    let window = Duration::from_secs(6);
    nodes.beat_at(&node("a"), 10_000).await.unwrap();
    nodes.beat_at(&node("b"), 12_000).await.unwrap();
    nodes.beat_at(&node("a"), 15_000).await.unwrap();
    assert_eq!(
        nodes.live_at(window, 17_000).await.unwrap(),
        [node("a"), node("b")]
    );
    // b stopped beating at 12 s; at 19 s it is gone.
    assert_eq!(nodes.live_at(window, 19_000).await.unwrap(), [node("a")]);
    nodes.leave(&node("a")).await.unwrap();
    assert!(nodes.live_at(window, 19_000).await.unwrap().is_empty());
}

#[tokio::test]
async fn one_holder_at_a_time_until_the_lease_expires() {
    let dir = tempfile::tempdir().unwrap();
    let leases = Leases::new(index(&dir).await);
    let ttl = Duration::from_secs(10);
    assert!(leases.acquire_at("search.feed", "a", ttl, 0).await.unwrap());
    assert!(
        !leases
            .acquire_at("search.feed", "b", ttl, 5_000)
            .await
            .unwrap()
    );
    // The holder renews.
    assert!(
        leases
            .acquire_at("search.feed", "a", ttl, 9_000)
            .await
            .unwrap()
    );
    assert!(
        !leases
            .acquire_at("search.feed", "b", ttl, 15_000)
            .await
            .unwrap()
    );
    // Expired: b takes over, and a is refused.
    assert!(
        leases
            .acquire_at("search.feed", "b", ttl, 19_001)
            .await
            .unwrap()
    );
    assert!(
        !leases
            .acquire_at("search.feed", "a", ttl, 19_002)
            .await
            .unwrap()
    );
    // Released: anyone may take it.
    leases.release("search.feed", "b").await.unwrap();
    assert!(
        leases
            .acquire_at("search.feed", "a", ttl, 19_003)
            .await
            .unwrap()
    );
    // Other names are independent.
    assert!(
        leases
            .acquire_at("work.daily", "b", ttl, 19_003)
            .await
            .unwrap()
    );
}
