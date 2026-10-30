// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Upgrades from every earlier schema (ADR 0022, D253; issue #57).
//!
//! Any earlier migration may be the one a previous release shipped, so for
//! each version the test stops there, writes the Northstar rows from
//! `tests/fixtures/upgrade/` that fit that schema, then opens the index,
//! which applies the remaining migrations. Every row must survive, and the
//! schema must end up exactly as a fresh install's. A migration that
//! rewrites existing tables adds a fixture at the version before it, so
//! its rewrite runs over real rows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers fail the test on any setup error"
)]

use std::fmt::Write as _;
use std::path::Path;
use std::str::FromStr as _;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnection, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{Connection as _, Executor as _};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::{Index, IndexConfig};

static SQLITE: Migrator = sqlx::migrate!("./migrations/sqlite");
static POSTGRES: Migrator = sqlx::migrate!("./migrations/postgres");

#[derive(Clone, Copy)]
enum Engine {
    Sqlite,
    Postgres,
}

/// The fixtures, as (version, SQL), oldest first.
fn fixtures() -> Vec<(i64, String)> {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/upgrade");
    let mut found: Vec<(i64, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "sql"))
        .map(|path| {
            let version =
                path.file_stem().unwrap().to_str().unwrap().parse().unwrap();
            (version, std::fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort_by_key(|(version, _)| *version);
    assert!(
        !found.is_empty(),
        "no upgrade fixtures in {}",
        dir.display()
    );
    found
}

/// Drops comment lines and replaces each `ID(n)` with the engine's
/// literal for a fixed ID.
fn render(sql: &str, engine: Engine) -> String {
    let code: String = sql
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .fold(String::new(), |mut acc, line| {
            acc.push_str(line);
            acc.push('\n');
            acc
        });
    let mut out = String::with_capacity(code.len());
    let mut rest = code.as_str();
    while let Some(start) = rest.find("ID(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let end = after.find(')').expect("ID( without a closing bracket");
        let n: u8 = after[..end].parse().expect("ID(n) takes a small number");
        let hex = format!("0193c0de0000700080000000000000{n:02x}");
        match engine {
            Engine::Sqlite => {
                let _ = write!(out, "X'{hex}'");
            }
            Engine::Postgres => {
                let _ = write!(
                    out,
                    "'{}-{}-{}-{}-{}'",
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                );
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The tables the fixtures up to `version` write to.
fn seeded_tables(version: i64) -> Vec<String> {
    let mut tables: Vec<String> = fixtures()
        .iter()
        .filter(|(v, _)| *v <= version)
        .flat_map(|(_, sql)| {
            sql.lines()
                .filter_map(|line| line.trim().strip_prefix("INSERT INTO "))
                .filter_map(|rest| rest.split_whitespace().next())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    tables.sort();
    tables.dedup();
    tables
}

fn versions(migrator: &Migrator) -> Vec<i64> {
    migrator.iter().map(|m| m.version).collect()
}

const SQLITE_SCHEMA: &str = "SELECT type || ' ' || name || ': ' || \
     coalesce(sql, '') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND \
     name <> '_sqlx_migrations' ORDER BY type, name";

const POSTGRES_SCHEMA: &str = "SELECT 'column ' || table_name || '.' || \
    column_name || ' ' || data_type || ' ' || is_nullable || ' ' || \
    coalesce(column_default, '') FROM information_schema.columns WHERE \
    table_schema = 'public' AND table_name <> '_sqlx_migrations' UNION ALL \
    SELECT 'index ' || indexdef FROM pg_indexes WHERE schemaname = 'public' \
    AND tablename <> '_sqlx_migrations' UNION ALL SELECT 'constraint ' || \
    conrelid::regclass || ' ' || conname || ' ' || pg_get_constraintdef(oid) \
    FROM pg_constraint WHERE connamespace = 'public'::regnamespace AND \
    conrelid::regclass::text <> '_sqlx_migrations' UNION ALL SELECT 'trigger ' \
    || tgrelid::regclass || ' ' || tgname || ' ' || pg_get_triggerdef(oid) \
    FROM pg_trigger WHERE NOT tgisinternal UNION ALL SELECT 'function ' || \
    p.proname || ' ' || md5(pg_get_functiondef(p.oid)) FROM pg_proc p WHERE \
    p.pronamespace = 'public'::regnamespace ORDER BY 1";

async fn sqlite_conn(path: &Path) -> SqliteConnection {
    let options = SqliteConnectOptions::from_str("sqlite://")
        .unwrap()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true);
    SqliteConnection::connect_with(&options).await.unwrap()
}

async fn sqlite_schema(path: &Path) -> Vec<String> {
    let mut conn = sqlite_conn(path).await;
    sqlx::query_scalar(SQLITE_SCHEMA)
        .fetch_all(&mut conn)
        .await
        .unwrap()
}

async fn count_sqlite(path: &Path, tables: &[String]) -> Vec<(String, i64)> {
    let mut conn = sqlite_conn(path).await;
    let mut counts = Vec::new();
    for table in tables {
        let sql = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&mut conn)
            .await
            .unwrap();
        counts.push((table.clone(), n));
    }
    counts
}

#[tokio::test]
async fn sqlite_upgrades_from_every_earlier_schema_with_data() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.sqlite");
    Index::open(&IndexConfig::sqlite(fresh.clone(), None))
        .await
        .unwrap()
        .close()
        .await;
    let want = sqlite_schema(&fresh).await;

    let all = versions(&SQLITE);
    for &from in &all[..all.len() - 1] {
        let path = dir.path().join(format!("from-{from}.sqlite"));
        let mut conn = sqlite_conn(&path).await;
        SQLITE.run_to(from, &mut conn).await.unwrap();
        for (_, sql) in fixtures().iter().filter(|(v, _)| *v <= from) {
            let _ = conn
                .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(render(
                    sql,
                    Engine::Sqlite,
                ))))
                .await
                .unwrap_or_else(|e| panic!("seeding at {from}: {e}"));
        }
        conn.close().await.unwrap();
        let tables = seeded_tables(from);
        let before = count_sqlite(&path, &tables).await;

        Index::open(&IndexConfig::sqlite(path.clone(), None))
            .await
            .unwrap_or_else(|e| panic!("upgrading from {from}: {e}"))
            .close()
            .await;

        assert_eq!(
            count_sqlite(&path, &tables).await,
            before,
            "rows lost upgrading from {from}"
        );
        assert_eq!(
            sqlite_schema(&path).await,
            want,
            "schema drift upgrading from {from}"
        );
    }
}

/// A server and a way to make empty databases on it: a scratch database
/// per call on `VELLUM_TEST_POSTGRES_URL` where containers cannot run,
/// otherwise a throwaway container.
struct Server {
    _container: Option<ContainerAsync<Postgres>>,
    admin_url: String,
}

impl Server {
    async fn start() -> Self {
        if let Ok(url) = std::env::var("VELLUM_TEST_POSTGRES_URL") {
            return Self {
                _container: None,
                admin_url: url,
            };
        }
        let container = Postgres::default()
            .with_tag("17-alpine")
            .start()
            .await
            .unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        Self {
            _container: Some(container),
            admin_url: format!(
                "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
            ),
        }
    }

    async fn database(&self, tag: &str) -> String {
        let name = format!("upgrade_{tag}_{}", std::process::id());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.admin_url)
            .await
            .unwrap();
        for sql in [
            format!("DROP DATABASE IF EXISTS {name}"),
            format!("CREATE DATABASE {name}"),
        ] {
            let _ = sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&admin)
                .await
                .unwrap();
        }
        admin.close().await;
        let base = self
            .admin_url
            .rsplit_once('/')
            .map_or(self.admin_url.as_str(), |(base, _)| base);
        format!("{base}/{name}")
    }
}

async fn postgres_schema(url: &str) -> Vec<String> {
    let mut conn = PgConnection::connect(url).await.unwrap();
    sqlx::query_scalar(POSTGRES_SCHEMA)
        .fetch_all(&mut conn)
        .await
        .unwrap()
}

async fn count_postgres(url: &str, tables: &[String]) -> Vec<(String, i64)> {
    let mut conn = PgConnection::connect(url).await.unwrap();
    let mut counts = Vec::new();
    for table in tables {
        let sql = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&mut conn)
            .await
            .unwrap();
        counts.push((table.clone(), n));
    }
    counts
}

#[tokio::test]
async fn postgres_upgrades_from_every_earlier_schema_with_data() {
    let server = Server::start().await;
    let fresh = server.database("fresh").await;
    Index::open(&IndexConfig::postgres(&fresh))
        .await
        .unwrap()
        .close()
        .await;
    let want = postgres_schema(&fresh).await;

    let all = versions(&POSTGRES);
    for &from in &all[..all.len() - 1] {
        let url = server.database(&format!("from_{from}")).await;
        let mut conn = PgConnection::connect(&url).await.unwrap();
        POSTGRES.run_to(from, &mut conn).await.unwrap();
        for (_, sql) in fixtures().iter().filter(|(v, _)| *v <= from) {
            let _ = conn
                .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(render(
                    sql,
                    Engine::Postgres,
                ))))
                .await
                .unwrap_or_else(|e| panic!("seeding at {from}: {e}"));
        }
        conn.close().await.unwrap();
        let tables = seeded_tables(from);
        let before = count_postgres(&url, &tables).await;

        Index::open(&IndexConfig::postgres(&url))
            .await
            .unwrap_or_else(|e| panic!("upgrading from {from}: {e}"))
            .close()
            .await;

        assert_eq!(
            count_postgres(&url, &tables).await,
            before,
            "rows lost upgrading from {from}"
        );
        assert_eq!(
            postgres_schema(&url).await,
            want,
            "schema drift upgrading from {from}"
        );
    }
}

#[test]
fn fixtures_render_for_both_engines() {
    let sql = "-- ID(n) in a comment is left alone\nVALUES (ID(1), ID(255))";
    assert_eq!(
        render(sql, Engine::Sqlite),
        "VALUES (X'0193c0de000070008000000000000001', \
            X'0193c0de0000700080000000000000ff')\n"
    );
    assert_eq!(
        render(sql, Engine::Postgres),
        "VALUES ('0193c0de-0000-7000-8000-000000000001', \
            '0193c0de-0000-7000-8000-0000000000ff')\n"
    );
    assert!(seeded_tables(3).contains(&"drive_nodes".to_owned()));
    assert!(seeded_tables(2).is_empty());
}
