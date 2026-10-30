// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Replica heartbeats and leases (ADR 0037) on the SQLite backend.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use std::time::Duration;

use vellum_db::cluster::{Cursors, Leases, Node, Nodes};
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

#[tokio::test]
async fn held_work_runs_on_one_holder_and_releases() {
    let dir = tempfile::tempdir().unwrap();
    let leases = Leases::new(index(&dir).await);
    let ttl = Duration::from_secs(30);
    // While a holds the lease, b's work does not run.
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let a = {
        let leases = leases.clone();
        tokio::spawn(async move {
            leases
                .hold("drive.rescan", "a", ttl, async move { rx.await.is_ok() })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        leases
            .hold("drive.rescan", "b", ttl, async { 1 })
            .await
            .unwrap(),
        None
    );
    tx.send(()).unwrap();
    assert_eq!(a.await.unwrap().unwrap(), Some(true));
    // a released it when its work ended.
    assert_eq!(
        leases
            .hold("drive.rescan", "b", ttl, async { 2 })
            .await
            .unwrap(),
        Some(2)
    );
}

#[tokio::test]
async fn only_the_lease_holder_moves_a_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let index = index(&dir).await;
    let (leases, cursors) = (Leases::new(index.clone()), Cursors::new(index));
    let ttl = Duration::from_secs(30);
    assert_eq!(cursors.get("search.drive_feed").await.unwrap(), None);
    assert!(leases.acquire("search.drive_feed", "a", ttl).await.unwrap());
    assert!(
        cursors
            .set("search.drive_feed", (1, 2), "search.drive_feed", "a")
            .await
            .unwrap()
    );
    assert!(
        !cursors
            .set("search.drive_feed", (9, 9), "search.drive_feed", "b")
            .await
            .unwrap()
    );
    assert_eq!(
        cursors.get("search.drive_feed").await.unwrap(),
        Some((1, 2))
    );
    assert!(
        cursors
            .set("search.drive_feed", (3, 0), "search.drive_feed", "a")
            .await
            .unwrap()
    );
    assert_eq!(
        cursors.get("search.drive_feed").await.unwrap(),
        Some((3, 0))
    );
}
