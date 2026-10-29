// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The replicas' change feed (ADR 0038) on the SQLite backend.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use vellum_db::feed::{Change, Feed};
use vellum_db::{Index, IndexConfig};

async fn feed(dir: &tempfile::TempDir) -> Feed {
    let index = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None))
        .await
        .unwrap();
    Feed::new(index)
}

fn subjects(changes: &[Change]) -> Vec<(i64, &str, &str)> {
    changes
        .iter()
        .map(|c| (c.seq, c.topic.as_str(), c.subject.as_str()))
        .collect()
}

#[tokio::test]
async fn a_reader_sees_every_change_past_its_cursor_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let feed = feed(&dir).await;
    assert_eq!(feed.head().await.unwrap(), 0);
    let start = feed.head().await.unwrap();

    let head = feed
        .publish(
            "a",
            &[("path", "spaces/eng/x.md"), ("path", "spaces/eng/y.md")],
        )
        .await
        .unwrap();
    assert_eq!(head, 2);
    assert_eq!(feed.publish("b", &[("spaces", "")]).await.unwrap(), 3);
    // Nothing to publish leaves the head alone.
    assert_eq!(feed.publish("b", &[]).await.unwrap(), 3);

    let page = feed.read(start, 2).await.unwrap();
    assert!(!page.reset);
    assert_eq!(
        subjects(&page.changes),
        [
            (1, "path", "spaces/eng/x.md"),
            (2, "path", "spaces/eng/y.md")
        ]
    );
    assert_eq!(page.changes[0].origin, "a");
    let page = feed.read(page.cursor, 10).await.unwrap();
    assert_eq!(subjects(&page.changes), [(3, "spaces", "")]);
    assert_eq!(page.changes[0].origin, "b");
    assert_eq!(page.cursor, 3);

    // Caught up: nothing new, same cursor.
    let page = feed.read(3, 10).await.unwrap();
    assert!(page.changes.is_empty() && !page.reset);
    assert_eq!(page.cursor, 3);
}

#[tokio::test]
async fn a_reader_behind_the_pruned_horizon_resets() {
    let dir = tempfile::tempdir().unwrap();
    let feed = feed(&dir).await;
    for i in 0..10 {
        let _ = feed
            .publish("a", &[("path", &format!("p{i}.md"))])
            .await
            .unwrap();
    }
    assert_eq!(feed.prune(4).await.unwrap(), 6);
    // Pruning again with the same window drops nothing.
    assert_eq!(feed.prune(4).await.unwrap(), 0);

    let behind = feed.read(3, 10).await.unwrap();
    assert!(behind.reset);
    assert!(behind.changes.is_empty());
    assert_eq!(behind.cursor, 10);

    // At the horizon itself nothing was missed.
    let edge = feed.read(6, 10).await.unwrap();
    assert!(!edge.reset);
    assert_eq!(edge.changes.first().map(|c| c.seq), Some(7));
    assert_eq!(edge.cursor, 10);
}

#[tokio::test]
async fn a_cursor_ahead_of_a_rebuilt_feed_resets() {
    let dir = tempfile::tempdir().unwrap();
    let feed = feed(&dir).await;
    let _ = feed.publish("a", &[("spaces", "")]).await.unwrap();
    // A cursor from an older, longer feed.
    let page = feed.read(500, 10).await.unwrap();
    assert!(page.reset);
    assert_eq!(page.cursor, 1);
}

#[tokio::test]
async fn concurrent_writers_never_leave_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let feed = feed(&dir).await;
    let mut tasks = Vec::new();
    for w in 0..8 {
        let feed = feed.clone();
        tasks.push(tokio::spawn(async move {
            for i in 0..25 {
                let subject = format!("w{w}/{i}.md");
                let _ = feed.publish("a", &[("path", &subject)]).await.unwrap();
            }
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let page = feed.read(0, 1000).await.unwrap();
    let seqs: Vec<i64> = page.changes.iter().map(|c| c.seq).collect();
    assert_eq!(seqs, (1..=200).collect::<Vec<_>>());
}
