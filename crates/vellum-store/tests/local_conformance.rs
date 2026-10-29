// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The local-folder adapter passes the storage conformance suite, bare and
//! wrapped in the metrics decorator.

use bytes::Bytes;
use futures::StreamExt;
use vellum_store::{
    BlobPath, BlobStore, LocalStore, Metered, Precondition, StoreError, conformance,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test(flavor = "multi_thread")]
async fn local_store_conforms() -> TestResult {
    let dir = tempfile::tempdir()?;
    conformance::run_all(&LocalStore::open(dir.path()).await?).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn metered_local_store_conforms() -> TestResult {
    let dir = tempfile::tempdir()?;
    conformance::run_all(&Metered::new(LocalStore::open(dir.path()).await?)).await?;
    Ok(())
}

#[tokio::test]
async fn temp_files_are_hidden_from_listings() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = LocalStore::open(dir.path()).await?;
    std::fs::write(dir.path().join(".vellum-tmp-123-0"), b"partial")?;
    let _ = store
        .put(
            &BlobPath::new("a.md")?,
            Bytes::from_static(b"x"),
            Precondition::None,
        )
        .await?;
    let listed: Vec<_> = store.list(None).collect().await;
    assert_eq!(listed.len(), 1);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_escape_the_root() -> TestResult {
    let outside = tempfile::tempdir()?;
    std::fs::write(outside.path().join("secret"), b"s")?;
    let dir = tempfile::tempdir()?;
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link"))?;
    let store = LocalStore::open(dir.path()).await?;
    let got = store.get(&BlobPath::new("link/secret")?).await;
    assert!(
        matches!(got, Err(StoreError::InvalidPath { .. })),
        "{got:?}"
    );
    let put = store
        .put(
            &BlobPath::new("link/new")?,
            Bytes::new(),
            Precondition::None,
        )
        .await;
    assert!(
        matches!(put, Err(StoreError::InvalidPath { .. })),
        "{put:?}"
    );
    assert_eq!(store.list(None).count().await, 0);
    Ok(())
}
