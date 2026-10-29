// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Postgres engine against a throwaway container: migrations from zero,
//! re-running them, and the shared conformance suite.

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_search::{PostgresBackend, conformance};

type Started = Result<(Option<ContainerAsync<Postgres>>, sqlx::PgPool), Box<dyn std::error::Error>>;

/// A fresh database: in a container, or in a scratch database on the server
/// named by `VELLUM_TEST_POSTGRES_URL` where containers cannot run.
async fn start() -> Started {
    if let Ok(url) = std::env::var("VELLUM_TEST_POSTGRES_URL") {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await?;
        let name = format!("search_{}", std::process::id());
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name}"
        )))
        .execute(&admin)
        .await?;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await?;
        let base = url.rsplit_once('/').map_or(url.as_str(), |(base, _)| base);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&format!("{base}/{name}"))
            .await?;
        return Ok((None, pool));
    }
    let container = Postgres::default().with_tag("17-alpine").start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await?;
    Ok((Some(container), pool))
}

#[tokio::test]
async fn postgres_conforms_and_migrations_are_idempotent() {
    let (_container, pool) = start().await.unwrap();
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
