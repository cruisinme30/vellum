// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Routing spaces to shards at runtime (ADR 0040 D174), over SQLite files.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use vellum_db::{IndexConfig, ShardRouter, ShardedIndex, ShardsConfig};
use vellum_kernel::ObjectId;

fn config(dir: &tempfile::TempDir) -> ShardsConfig {
    let db = |name: &str| {
        IndexConfig::sqlite(dir.path().join(format!("{name}.sqlite")), None)
    };
    ShardsConfig {
        main: db("main"),
        shards: vec![("eu".into(), db("eu")), ("us".into(), db("us"))],
    }
}

const fn space(n: u128) -> ObjectId {
    ObjectId::from_uuid(uuid::Uuid::from_u128(n))
}

#[tokio::test]
async fn new_spaces_spread_and_every_replica_agrees() {
    let dir = tempfile::tempdir().unwrap();
    let a = ShardRouter::open(ShardedIndex::open(&config(&dir)).await.unwrap())
        .await
        .unwrap();
    let b = ShardRouter::open(ShardedIndex::open(&config(&dir)).await.unwrap())
        .await
        .unwrap();

    // Unplaced spaces read from home.
    assert_eq!(a.position(space(1)).await.unwrap(), 0);

    let placed: Vec<usize> = {
        let mut out = Vec::new();
        for n in 1..=4 {
            out.push(a.place(space(n)).await.unwrap());
        }
        out
    };
    assert_eq!(
        placed,
        [0, 1, 0, 1],
        "the emptiest shard takes each new space"
    );

    // Another replica finds them, and placing again changes nothing.
    for (n, at) in (1..=4).zip(&placed) {
        assert_eq!(b.position(space(n)).await.unwrap(), *at);
        assert_eq!(b.place(space(n)).await.unwrap(), *at);
    }
    assert_eq!(a.len(), 2);
    assert_eq!(a.indexes().count(), 2);
}

#[tokio::test]
async fn an_entry_written_first_wins() {
    let dir = tempfile::tempdir().unwrap();
    let sharded = ShardedIndex::open(&config(&dir)).await.unwrap();
    let router = ShardRouter::open(sharded.clone()).await.unwrap();
    // Another replica placed the space on "us" before this one looked.
    sharded.claim(space(7), "us").await.unwrap();
    assert_eq!(router.place(space(7)).await.unwrap(), 1);
    // A later claim does not move it.
    sharded.claim(space(7), "eu").await.unwrap();
    assert_eq!(
        sharded.assigned(space(7)).await.unwrap().as_deref(),
        Some("us")
    );
}

#[tokio::test]
async fn spaces_written_before_sharding_stay_where_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let sharded = ShardedIndex::open(&config(&dir)).await.unwrap();
    // A space already on "us" that the map does not know.
    let vellum_db::IndexPool::Sqlite(pool) = sharded.shards()[1].index().pool()
    else {
        panic!("sqlite")
    };
    let _ = sqlx::query(
        "INSERT INTO spaces (space_id, kind, slug, root_path, name, \
         indexed_at) VALUES ($1, 'shared_drive', 'old', 'drives/shared/old', \
         'Old', 0)",
    )
    .bind(space(9).as_uuid())
    .execute(pool)
    .await
    .unwrap();
    let router = ShardRouter::open(sharded).await.unwrap();
    assert_eq!(router.position(space(9)).await.unwrap(), 1);
    assert_eq!(router.place(space(9)).await.unwrap(), 1);
}

#[tokio::test]
async fn one_database_keeps_every_space_at_home_without_a_map() {
    let dir = tempfile::tempdir().unwrap();
    let index = vellum_db::Index::open(&IndexConfig::sqlite(
        dir.path().join("one.sqlite"),
        None,
    ))
    .await
    .unwrap();
    let sharded = ShardedIndex::single(index);
    let router = ShardRouter::open(sharded.clone()).await.unwrap();
    assert_eq!(router.place(space(1)).await.unwrap(), 0);
    assert_eq!(router.position(space(2)).await.unwrap(), 0);
    assert!(
        sharded.counts().await.unwrap().is_empty(),
        "nothing recorded"
    );
}
