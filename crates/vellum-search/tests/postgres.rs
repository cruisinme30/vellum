// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Postgres engine against a throwaway container: migrations from zero,
//! re-running them, the shared conformance suite, and the drive feed's
//! queries on the Postgres dialect.

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use uuid::Uuid;
use vellum_db::changes::{DriveChanges, Position};
use vellum_db::{Index, IndexConfig, IndexPool};
use vellum_kernel::{DocId, ObjectId, SpaceId};
use vellum_search::{DocumentSource as _, DriveSource, PostgresBackend, conformance};
use vellum_store::{BlobStore, LocalStore};

type Started = Result<(Option<ContainerAsync<Postgres>>, String), Box<dyn std::error::Error>>;

/// A fresh database's URL: in a container, or a scratch database named
/// `search_<tag>_<pid>` on the server named by `VELLUM_TEST_POSTGRES_URL`
/// where containers cannot run.
async fn start(tag: &str) -> Started {
    if let Ok(url) = std::env::var("VELLUM_TEST_POSTGRES_URL") {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await?;
        let name = format!("search_{tag}_{}", std::process::id());
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name}"
        )))
        .execute(&admin)
        .await?;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await?;
        let base = url.rsplit_once('/').map_or(url.as_str(), |(base, _)| base);
        return Ok((None, format!("{base}/{name}")));
    }
    let container = Postgres::default().with_tag("17-alpine").start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    Ok((Some(container), url))
}

async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(4).connect(url).await
}

#[tokio::test]
async fn postgres_conforms_and_migrations_are_idempotent() {
    let (_container, url) = start("engine").await.unwrap();
    let pool = connect(&url).await.unwrap();
    let backend = PostgresBackend::connect(pool.clone()).await.unwrap();
    // A second start finds the schema current.
    let _again = PostgresBackend::connect(pool.clone()).await.unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'vellum_search' ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tables, ["_sqlx_migrations", "blocks", "docs"]);
    conformance::run(Arc::new(backend)).await.unwrap();
}

#[tokio::test]
async fn drive_feed_reads_pages_from_postgres() {
    let (_container, url) = start("feed").await.unwrap();
    let index = Arc::new(Index::open(&IndexConfig::postgres(url)).await.unwrap());
    let IndexPool::Postgres(pool) = index.pool() else {
        panic!("expected Postgres");
    };
    let (space, folder, page) = (0x10_u128, 0x20_u128, 0x30_u128);
    for (id, kind, parent, path) in [
        (folder, "folder", None, "eng"),
        (page, "page", Some(folder), "eng/launch.md"),
    ] {
        let _ = sqlx::query(
            "INSERT INTO drive_nodes (space_id, node_id, kind, parent_id, tree_parent_id, \
             path, title, byte_size, modified_at, created_by, trash) \
             VALUES ($1, $2, $3, $4, $4, $5, 'Launch', 0, 1000, NULL, NULL)",
        )
        .bind(Uuid::from_u128(space))
        .bind(Uuid::from_u128(id))
        .bind(kind)
        .bind(parent.map(Uuid::from_u128))
        .bind(path)
        .execute(pool)
        .await
        .unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("eng")).unwrap();
    std::fs::write(dir.path().join("eng/launch.md"), "Ship on Friday.\n").unwrap();
    let store: Arc<dyn BlobStore> = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let source = DriveSource::new(Arc::clone(&index), store);

    let doc = DocId::from_u128(page);
    assert_eq!(source.list().await.unwrap(), vec![doc]);
    let loaded = source.load(doc).await.unwrap().unwrap();
    assert_eq!(
        loaded.ancestors,
        vec![
            ObjectId::from(SpaceId::from_u128(space)),
            ObjectId::from_u128(folder)
        ]
    );
    assert_eq!(loaded.blocks.len(), 1);
    // The insert was logged by the trigger, and committed, so it is
    // visible past a position from before it.
    let log = DriveChanges::new((*index).clone());
    let batch = log.read(Position::default(), 10).await.unwrap();
    assert_eq!(batch.pages, vec![Uuid::from_u128(page)]);
    assert!(batch.next > Position::default());
    assert!(log.read(batch.next, 10).await.unwrap().pages.is_empty());
}
