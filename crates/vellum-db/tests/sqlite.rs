// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SQLite backend: migrations from zero and SQLCipher encryption.

use vellum_db::{Backend, Error, Index, IndexConfig, IndexKey};

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
