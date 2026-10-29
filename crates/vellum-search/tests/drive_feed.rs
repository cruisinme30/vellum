// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end: a page saved in the drive becomes searchable, an edit
//! replaces its text, and a deleted page drops out, with no event wiring
//! beyond the feed itself.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use vellum_db::{Index, IndexConfig, IndexPool, sqlx};
use vellum_kernel::{DocId, SpaceId, UserId};
use vellum_search::conformance::{StaticAccess, StaticStars};
use vellum_search::{
    DriveFeed, DriveSource, FeedConfig, Indexer, IndexerConfig, Location, Principal, SearchQuery,
    SearchService, Start, TantivyBackend, TantivyConfig,
};
use vellum_store::{BlobStore, LocalStore};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const SPACE: u128 = 0x10;
const PAGE: u128 = 0x30;
const ADA: u128 = 0x40;

const fn pool(index: &Index) -> Result<&sqlx::SqlitePool, &'static str> {
    match index.pool() {
        IndexPool::Sqlite(pool) => Ok(pool),
        _ => Err("expected SQLite"),
    }
}

async fn titles(service: &SearchService, text: &str) -> Vec<String> {
    let ada = Principal::User(UserId::from_u128(ADA));
    service
        .search(&ada, &SearchQuery::new(text, 10))
        .await
        .map(|page| page.hits.into_iter().map(|hit| hit.title).collect())
        .unwrap_or_default()
}

/// Polls until `text` finds `want` hits, for at most five seconds.
async fn eventually(service: &SearchService, text: &str, want: usize) -> Vec<String> {
    for _ in 0..250 {
        let found = titles(service, text).await;
        if found.len() == want {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    titles(service, text).await
}

#[tokio::test]
async fn saved_pages_flow_into_search() -> TestResult {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("files");
    let index =
        Arc::new(Index::open(&IndexConfig::sqlite(dir.path().join("index.db"), None)).await?);
    let store: Arc<dyn BlobStore> = Arc::new(LocalStore::open(&root).await?);

    let (backend, _) = TantivyBackend::open(&TantivyConfig {
        location: Location::Memory,
        key: None,
        model: None,
    })?;
    let mut access = StaticAccess::default();
    access.grant(
        Principal::User(UserId::from_u128(ADA)),
        &[SpaceId::from_u128(SPACE)],
        &[DocId::from_u128(PAGE)],
    );
    let service = SearchService::new(
        Arc::new(backend),
        Arc::new(access),
        Arc::new(StaticStars::default()),
    );
    let source = Arc::new(DriveSource::new(Arc::clone(&index), store));
    let (indexer, _task) = Indexer::spawn(service.clone(), source, IndexerConfig::default());
    let feed = DriveFeed::spawn(
        Arc::clone(&index),
        indexer,
        FeedConfig {
            interval: Duration::from_millis(20),
            start: Start::CatchUp,
        },
    );

    // Save a page the way the drive does: the file, then its row.
    std::fs::create_dir_all(root.join("eng"))?;
    std::fs::write(
        root.join("eng/launch.md"),
        "# Launch\n\nShip the rocket on Friday.\n",
    )?;
    let _ = sqlx::query(
        "INSERT INTO drive_nodes (space_id, node_id, kind, parent_id, tree_parent_id, path, \
         title, byte_size, modified_at, created_by, trash) \
         VALUES ($1, $2, 'page', NULL, NULL, 'eng/launch.md', 'Launch plan', 40, 1000, $3, NULL)",
    )
    .bind(Uuid::from_u128(SPACE))
    .bind(Uuid::from_u128(PAGE))
    .bind(Uuid::from_u128(ADA))
    .execute(pool(&index)?)
    .await?;
    assert_eq!(eventually(&service, "rocket", 1).await, vec!["Launch plan"]);

    // An edit replaces the old text.
    std::fs::write(
        root.join("eng/launch.md"),
        "# Launch\n\nShip the satellite instead.\n",
    )?;
    let _ = sqlx::query("UPDATE drive_nodes SET modified_at = 2000, byte_size = 41")
        .execute(pool(&index)?)
        .await?;
    assert_eq!(
        eventually(&service, "satellite", 1).await,
        vec!["Launch plan"]
    );
    assert!(eventually(&service, "rocket", 0).await.is_empty());

    // A purge removes it.
    let _ = sqlx::query("DELETE FROM drive_nodes")
        .execute(pool(&index)?)
        .await?;
    assert!(eventually(&service, "satellite", 0).await.is_empty());

    feed.abort();
    Ok(())
}
