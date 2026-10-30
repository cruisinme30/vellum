// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The update log in the index database: one conformance suite, run on
//! SQLite (sealed and in clear) and on Postgres in a throwaway container,
//! plus a room rebuilt on a second hub from the same database.

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use crate::support;

use std::sync::Arc;
use std::time::Duration;

use support::{Editor, TestCodec, add_paragraph, room};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_collab::memory::MemoryFiles;
use vellum_collab::sql::{Envelope, Plaintext, SqlUpdateStore};
use vellum_collab::{Access, CollabConfig, Hub, StoredUpdate, UpdateStore};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_db::{Index, IndexConfig, IndexPool, sqlx};

fn keyring() -> LocalKeyring {
    LocalKeyring::new(KekId::new("test").unwrap(), SecretKey::generate().unwrap())
}

async fn sqlite(dir: &tempfile::TempDir) -> Index {
    Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None))
        .await
        .unwrap()
}

/// Everything the room relies on, against any backend.
async fn conformance(store: &SqlUpdateStore) {
    let a = room();
    let b = room();

    // An empty log; seeding writes once.
    assert!(store.load(a).await.unwrap().is_empty());
    assert!(store.seed(a, b"seed".to_vec()).await.unwrap());
    assert!(!store.seed(a, b"again".to_vec()).await.unwrap());

    // Appends follow in order; another document has its own log.
    assert_eq!(store.append(a, b"one".to_vec()).await.unwrap(), 2);
    assert_eq!(store.append(a, b"two".to_vec()).await.unwrap(), 3);
    assert_eq!(store.append(b, b"other".to_vec()).await.unwrap(), 1);
    assert!(!store.seed(b, b"late".to_vec()).await.unwrap());
    let log = store.load(a).await.unwrap();
    let records: Vec<(u64, &[u8])> = log.iter().map(|r| (r.seq, r.update.as_slice())).collect();
    assert_eq!(
        records,
        [(1, &b"seed"[..]), (2, &b"one"[..]), (3, &b"two"[..])]
    );

    // Compaction folds the prefix into one record and keeps the tail.
    assert_eq!(store.append(a, b"three".to_vec()).await.unwrap(), 4);
    store.compact(a, 3, b"merged".to_vec()).await.unwrap();
    assert_eq!(
        store.load(a).await.unwrap(),
        [
            StoredUpdate {
                seq: 3,
                update: b"merged".to_vec()
            },
            StoredUpdate {
                seq: 4,
                update: b"three".to_vec()
            },
        ]
    );
    // New records still go after the highest one.
    assert_eq!(store.append(a, b"four".to_vec()).await.unwrap(), 5);
    store.compact(a, 5, b"all".to_vec()).await.unwrap();
    assert_eq!(store.load(a).await.unwrap().len(), 1);
    assert_eq!(store.append(a, b"five".to_vec()).await.unwrap(), 6);

    // The other document never saw any of it.
    assert_eq!(store.load(b).await.unwrap().len(), 1);
}

#[tokio::test]
async fn sqlite_log_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let index = sqlite(&dir).await;
    conformance(&SqlUpdateStore::new(index.clone(), Envelope(keyring()))).await;
    index.close().await;
}

#[tokio::test]
async fn sqlite_log_in_clear() {
    let dir = tempfile::tempdir().unwrap();
    let index = sqlite(&dir).await;
    conformance(&SqlUpdateStore::new(index.clone(), Plaintext)).await;
    index.close().await;
}

#[tokio::test]
async fn sealed_records_are_unreadable_without_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let index = sqlite(&dir).await;
    let key = room();
    let sealed = SqlUpdateStore::new(index.clone(), Envelope(keyring()));
    let _seq = sealed
        .append(key, b"Northstar launch plan".to_vec())
        .await
        .unwrap();

    // The stored bytes never contain the text.
    let IndexPool::Sqlite(pool) = index.pool() else {
        unreachable!()
    };
    let stored: Vec<u8> = sqlx::query_scalar("SELECT payload FROM collab_updates")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(!stored.windows(9).any(|w| w == b"Northstar"));

    // Another key, or none, cannot read it.
    let other = SqlUpdateStore::new(index.clone(), Envelope(keyring()));
    assert!(other.load(key).await.is_err());
    assert!(
        SqlUpdateStore::new(index.clone(), Plaintext)
            .load(key)
            .await
            .is_err()
    );
    index.close().await;
}

#[tokio::test]
async fn a_record_moved_to_another_document_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let index = sqlite(&dir).await;
    let store = SqlUpdateStore::new(index.clone(), Envelope(keyring()));
    let (from, to) = (room(), room());
    let _seq = store.append(from, b"secret".to_vec()).await.unwrap();

    let IndexPool::Sqlite(pool) = index.pool() else {
        unreachable!()
    };
    let _moved = sqlx::query("UPDATE collab_updates SET space_id = ?, doc_id = ?")
        .bind(to.space.as_u128().to_be_bytes().to_vec())
        .bind(to.doc.as_u128().to_be_bytes().to_vec())
        .execute(pool)
        .await
        .unwrap();
    assert!(store.load(to).await.is_err());
    index.close().await;
}

#[tokio::test]
async fn a_second_hub_rebuilds_the_room_from_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let config = || CollabConfig {
        batch_window: Duration::from_millis(10),
        ..CollabConfig::default()
    };
    let files = Arc::new(MemoryFiles::new());
    let hub = |index: Index| {
        Hub::new(
            Arc::new(SqlUpdateStore::new(index, Plaintext)),
            files.clone(),
            Arc::new(TestCodec),
            config(),
        )
    };
    let key = room();
    files.put(key, "Seed\n");

    let first_index = sqlite(&dir).await;
    let first = hub(first_index.clone());
    let mut alice = Editor::join(&first, key, Access::Write).await;
    assert!(alice.pump_until(|e| e.markdown() == "Seed\n").await);
    alice.edit(|doc| add_paragraph(doc, "typed")).await;
    alice.pump().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A different process opens the same database.
    let second_index = sqlite(&dir).await;
    let second = hub(second_index.clone());
    let mut bob = Editor::join(&second, key, Access::Write).await;
    assert!(
        bob.pump_until(|e| e.markdown() == "Seed\n\ntyped\n").await,
        "{}",
        bob.markdown()
    );

    // Stop the rooms and the database threads before the process exits,
    // so none of them is still inside SQLCipher during exit-time cleanup.
    first.shutdown().await;
    second.shutdown().await;
    first_index.close().await;
    second_index.close().await;
}

type Started = Result<(ContainerAsync<Postgres>, String), Box<dyn std::error::Error>>;

async fn postgres() -> Started {
    let container = Postgres::default().with_tag("17-alpine").start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    Ok((
        container,
        format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"),
    ))
}

#[tokio::test]
async fn postgres_log_sealed() {
    let (_container, url) = postgres().await.unwrap();
    let index = Index::open(&IndexConfig::postgres(&url)).await.unwrap();
    let store = SqlUpdateStore::new(index.clone(), Envelope(keyring()));
    conformance(&store).await;

    // Two writers racing for the same position: one wins, none is lost.
    let key = room();
    let (left, right) = tokio::join!(
        store.append(key, b"left".to_vec()),
        store.append(key, b"right".to_vec())
    );
    let won = [left.is_ok(), right.is_ok()]
        .iter()
        .filter(|ok| **ok)
        .count();
    assert!(won >= 1);
    assert_eq!(store.load(key).await.unwrap().len(), won);
    index.close().await;
}
