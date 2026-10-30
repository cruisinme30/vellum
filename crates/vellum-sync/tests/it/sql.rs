// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The SQL sync store against the conformance suite, on SQLite and on
//! Postgres in a throwaway container.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail loudly"
)]

use std::sync::Mutex;

use serde_json::json;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ImageExt as _;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use vellum_db::{Index, IndexConfig};
use vellum_kernel::{SpaceId, UserId};
use vellum_sync::conformance::check_store;
use vellum_sync::{Change, ClientId, SqlStore, SyncStore as _};

/// The conformance suite wants a fresh store per case and builds it
/// synchronously, so the stores are opened up front.
async fn suite(stores: Vec<SqlStore>) {
    let stores = Mutex::new(stores);
    check_store(|| stores.lock().unwrap().pop().expect("enough stores"))
        .await
        .unwrap();
}

async fn sqlite(dir: &tempfile::TempDir, name: &str) -> SqlStore {
    let config = IndexConfig::sqlite(dir.path().join(name), None);
    SqlStore::new(Index::open(&config).await.unwrap())
}

#[tokio::test]
async fn sqlite_store_conforms() {
    let dir = tempfile::tempdir().unwrap();
    let mut stores = Vec::new();
    for n in 0..4 {
        stores.push(sqlite(&dir, &format!("{n}.sqlite")).await);
    }
    suite(stores).await;
}

#[tokio::test]
async fn sqlite_store_survives_reopen_and_prunes() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite(&dir, "index.sqlite").await;
    let space = SpaceId::generate();
    store
        .append(vec![
            Change::put(space, "t", "a", json!(1)),
            Change::delete(space, "t", "b"),
            Change::put(space, "t", "c", json!(3)),
        ])
        .await
        .unwrap();
    let client = ClientId::parse("device-1").unwrap();
    store
        .advance_client(&client, UserId::generate(), u64::MAX >> 1)
        .await
        .unwrap();
    let epoch = store.info().await.unwrap().epoch;

    let store = sqlite(&dir, "index.sqlite").await;
    assert_eq!(
        store.info().await.unwrap().epoch,
        epoch,
        "the epoch is stable"
    );
    store.prune_tombstones(2).await.unwrap();
    let keys: Vec<_> = store
        .read(0, 10, false)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.seq, r.change.key))
        .collect();
    assert_eq!(keys, vec![(1, "a".to_owned()), (3, "c".to_owned())]);
    assert_eq!(store.info().await.unwrap().horizon, 2);
    store.prune_tombstones(1).await.unwrap();
    assert_eq!(store.info().await.unwrap().horizon, 2, "never lowered");
    // The next write keeps counting from the old head.
    store
        .append(vec![Change::put(space, "t", "d", json!(4))])
        .await
        .unwrap();
    assert_eq!(store.read(3, 10, false).await.unwrap()[0].seq, 4);

    let err = store
        .advance_client(&client, UserId::generate(), u64::MAX)
        .await
        .unwrap_err();
    assert_eq!(err.kind().code(), "invalid_input", "{err}");
}

#[tokio::test]
async fn sqlite_appends_from_many_tasks_keep_every_position() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite(&dir, "index.sqlite").await;
    let space = SpaceId::generate();
    let mut tasks = Vec::new();
    for n in 0..16 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .append(vec![Change::put(space, "t", &n.to_string(), json!(n))])
                .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    let seqs: Vec<_> = store
        .read(0, 100, false)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(seqs, (1..=16).collect::<Vec<_>>());
}

#[tokio::test]
async fn postgres_store_conforms() {
    let container = Postgres::default()
        .with_tag("17-alpine")
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let base = format!("postgres://postgres:postgres@127.0.0.1:{port}");
    let admin = Index::open(&IndexConfig::postgres(format!("{base}/postgres")))
        .await
        .unwrap();
    let mut stores = Vec::new();
    for n in 0..4 {
        let name = format!("sync_{n}");
        if let vellum_db::IndexPool::Postgres(pool) = admin.pool() {
            let _ = vellum_db::sqlx::query(vellum_db::sqlx::AssertSqlSafe(
                format!("CREATE DATABASE {name}"),
            ))
            .execute(pool)
            .await
            .unwrap();
        }
        let config = IndexConfig::postgres(format!("{base}/{name}"));
        stores.push(SqlStore::new(Index::open(&config).await.unwrap()));
    }
    suite(stores).await;
}
