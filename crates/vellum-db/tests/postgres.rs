// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Postgres backend in a throwaway container: migrations from zero,
//! schema parity with SQLite, and schema constraints.

mod common;

use sqlx::Connection as _;
use sqlx::postgres::PgConnection;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::{Backend, Index, IndexConfig, PlainFiles};

type Started =
    Result<(ContainerAsync<Postgres>, String), Box<dyn std::error::Error>>;

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
    let sqlite =
        Index::open(&IndexConfig::sqlite(dir.path().join("i.sqlite"), None))
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

#[tokio::test]
async fn schema_enforces_paths_and_cascades() {
    let (_container, url) = start().await.unwrap();
    Index::open(&IndexConfig::postgres(&url))
        .await
        .unwrap()
        .close()
        .await;
    let mut conn = PgConnection::connect(&url).await.unwrap();

    let _ = sqlx::query(
        "INSERT INTO spaces (space_id, kind, slug, root_path, name, \
         indexed_at) VALUES ('01900000-0000-7000-8000-000000000001', 'team', \
         'eng', 'teams/ENG', 'ENG', 0)",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    let insert_doc = "INSERT INTO documents (space_id, doc_id, kind, path, \
                      title, content_hash, byte_size, modified_at, indexed_at) \
                      VALUES ('01900000-0000-7000-8000-000000000001', \
                      $1::uuid, 'issue', 'issues/ENG-1.md', 'First', 'h', 1, \
                      0, 0)";
    let _ = sqlx::query(insert_doc)
        .bind("01900000-0000-7000-8000-000000000002")
        .execute(&mut conn)
        .await
        .unwrap();
    let dup =
        sqlx::query(insert_doc).bind("01900000-0000-7000-8000-000000000003");
    assert!(
        dup.execute(&mut conn).await.is_err(),
        "duplicate path in a space must be rejected"
    );

    let _ = sqlx::query("DELETE FROM spaces")
        .execute(&mut conn)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn reindex_writes_the_same_rows_as_on_sqlite() {
    let (_container, url) = start().await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let index = Index::open(&IndexConfig::postgres(&url)).await.unwrap();
    for _ in 0..2 {
        let report = index.reindex(work.path(), PlainFiles).await.unwrap();
        assert_eq!(
            (report.spaces, report.folders, report.documents),
            (3, 1, 9)
        );
        assert_eq!(report.problems.len(), 3);
    }

    let mut conn = PgConnection::connect(&url).await.unwrap();
    let depth: i32 = sqlx::query_scalar(
        "SELECT depth FROM page_ancestors WHERE ancestor_id = $1 AND \
            descendant_id = $2",
    )
    .bind(uuid::Uuid::from_u128(fx.eng_space.as_u128()))
    .bind(uuid::Uuid::from_u128(fx.handbook_intro.as_u128()))
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(depth, 2);
}

/// Two replicas writing at once (ADR 0038): positions still commit in
/// order, so a reader polling meanwhile never steps over one.
#[tokio::test]
async fn feed_positions_commit_in_order_under_concurrent_writers() {
    use vellum_db::feed::Feed;

    let (_container, url) = start().await.unwrap();
    let feed =
        Feed::new(Index::open(&IndexConfig::postgres(&url)).await.unwrap());
    let mut writers = Vec::new();
    for w in 0..6 {
        let feed = feed.clone();
        writers.push(tokio::spawn(async move {
            for i in 0..40 {
                let subject = format!("w{w}/{i}.md");
                let _ = feed
                    .publish(
                        &format!("r{w}"),
                        &[("path", &subject), ("path", "x")],
                    )
                    .await
                    .unwrap();
            }
        }));
    }
    // Follow while they write: every page must start right after the cursor.
    let mut cursor = 0;
    let mut seen = Vec::new();
    while seen.len() < 480 {
        let page = feed.read(cursor, 64).await.unwrap();
        assert!(!page.reset);
        for c in &page.changes {
            assert_eq!(c.seq, cursor + 1, "skipped a position");
            cursor = c.seq;
            seen.push(c.seq);
        }
        cursor = page.cursor;
        tokio::task::yield_now().await;
    }
    for w in writers {
        w.await.unwrap();
    }
    assert_eq!(seen, (1..=480).collect::<Vec<_>>());
}
