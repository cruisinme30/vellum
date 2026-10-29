// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SQLite backend: migrations from zero, SQLCipher encryption, schema constraints.

use std::str::FromStr as _;

use sqlx::Connection as _;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use vellum_db::{Backend, Error, Index, IndexConfig, IndexKey};

const TABLES: [&str; 8] = [
    "documents",
    "effective_access",
    "folders",
    "grants",
    "index_meta",
    "page_ancestors",
    "spaces",
    "user_doc_state",
];

const fn key(byte: u8) -> IndexKey {
    IndexKey::new([byte; 32])
}

#[tokio::test]
async fn migrates_from_zero_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let config = IndexConfig::sqlite(dir.path().join("index.sqlite"), Some(key(7)));
    let index = Index::open(&config).await.unwrap();
    assert_eq!(index.backend(), Backend::Sqlite);
    index.ping().await.unwrap();
    index.close().await;

    // Reopening an existing index re-checks migrations and applies nothing new.
    let index = Index::open(&config).await.unwrap();
    index.migrate().await.unwrap();
    index.close().await;
}

#[tokio::test]
async fn encrypted_file_is_unreadable_without_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    let index = Index::open(&IndexConfig::sqlite(path.clone(), Some(key(1))))
        .await
        .unwrap();
    index.close().await;

    let header = std::fs::read(&path).unwrap();
    assert!(
        !header.starts_with(b"SQLite format 3"),
        "index must be encrypted at rest"
    );

    for wrong in [Some(key(2)), None] {
        let err = Index::open(&IndexConfig::sqlite(path.clone(), wrong))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::Locked { .. }),
            "unexpected error: {err}"
        );
    }
}

#[tokio::test]
async fn plaintext_mode_writes_a_plain_sqlite_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    let index = Index::open(&IndexConfig::sqlite(path.clone(), None))
        .await
        .unwrap();
    assert_eq!(index.table_names().await.unwrap(), TABLES);
    index.close().await;
    assert!(
        std::fs::read(&path)
            .unwrap()
            .starts_with(b"SQLite format 3")
    );
}

#[tokio::test]
async fn schema_enforces_ids_paths_and_cascades() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    Index::open(&IndexConfig::sqlite(path.clone(), None))
        .await
        .unwrap()
        .close()
        .await;

    let options = SqliteConnectOptions::from_str("sqlite://")
        .unwrap()
        .filename(&path)
        .foreign_keys(true);
    let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
    let space = [1_u8; 16];
    let doc = [2_u8; 16];

    let _ = sqlx::query(
        "INSERT INTO spaces (space_id, kind, slug, root_path, name, indexed_at) \
         VALUES (?, 'shared_drive', 'eng', 'drives/shared/eng', 'Engineering', 0)",
    )
    .bind(&space[..])
    .execute(&mut conn)
    .await
    .unwrap();

    let insert_doc = "INSERT INTO documents (space_id, doc_id, kind, path, title, content_hash, \
                      byte_size, modified_at, indexed_at) VALUES (?, ?, 'page', ?, 'Roadmap', 'h', 1, 0, 0)";
    let _ = sqlx::query(insert_doc)
        .bind(&space[..])
        .bind(&doc[..])
        .bind("roadmap.md")
        .execute(&mut conn)
        .await
        .unwrap();

    // Same path twice in one space, a short ID, and an unknown space are all rejected.
    let dup_path = sqlx::query(insert_doc)
        .bind(&space[..])
        .bind(&[3_u8; 16][..])
        .bind("roadmap.md");
    assert!(dup_path.execute(&mut conn).await.is_err());
    let short_id = sqlx::query(insert_doc)
        .bind(&space[..])
        .bind(&[4_u8; 8][..])
        .bind("other.md");
    assert!(short_id.execute(&mut conn).await.is_err());
    let no_space = sqlx::query(insert_doc)
        .bind(&[9_u8; 16][..])
        .bind(&[5_u8; 16][..])
        .bind("x.md");
    assert!(no_space.execute(&mut conn).await.is_err());

    // Deleting a space removes its derived rows.
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
