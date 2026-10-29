// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Postgres backend in a throwaway container: migrations from zero and
//! schema parity with SQLite.

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::{Backend, Index, IndexConfig};

type Started = Result<(ContainerAsync<Postgres>, String), Box<dyn std::error::Error>>;

async fn start() -> Started {
    let container = Postgres::default().with_tag("17-alpine").start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    Ok((container, url))
}

#[tokio::test]
async fn migrates_from_zero_with_the_same_tables_as_sqlite() {
    let (_container, url) = start().await.unwrap();
    let index = Index::open(&IndexConfig::postgres(&url)).await.unwrap();
    assert_eq!(index.backend(), Backend::Postgres);
    index.ping().await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let sqlite = Index::open(&IndexConfig::sqlite(dir.path().join("i.sqlite"), None))
        .await
        .unwrap();
    assert_eq!(
        index.table_names().await.unwrap(),
        sqlite.table_names().await.unwrap()
    );

    index.migrate().await.unwrap();
    index.close().await;
    Index::open(&IndexConfig::postgres(&url))
        .await
        .unwrap()
        .close()
        .await;
}
