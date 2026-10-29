// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Storage adapter conformance suite.
//!
//! Every [`BlobStore`] adapter (local, S3, Google Drive, Dropbox, `OneDrive`,
//! Git) and every decorator (metrics, encryption) must pass [`run_all`]:
//!
//! ```ignore
//! vellum_store::conformance::run_all(&my_store).await?;
//! ```
//!
//! Checks write only beneath `conformance/`, which must start empty.

use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};

use crate::{BlobPath, BlobStore, Precondition, StoreError};

/// A failed conformance check.
#[derive(Debug, thiserror::Error)]
#[error("conformance check `{check}` failed: {detail}")]
pub struct Failure {
    /// Name of the check.
    pub check: &'static str,
    /// What went wrong.
    pub detail: String,
}

type Outcome = Result<(), Failure>;

macro_rules! ensure {
    ($check:expr, $cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(Failure { check: $check, detail: format!($($msg)+) });
        }
    };
}

fn p(check: &'static str, rel: &str) -> Result<BlobPath, Failure> {
    BlobPath::new(format!("conformance/{check}/{rel}")).map_err(|e| Failure {
        check,
        detail: e.to_string(),
    })
}

fn fail(check: &'static str) -> impl Fn(StoreError) -> Failure {
    move |e| Failure {
        check,
        detail: format!("unexpected error: {e}"),
    }
}

/// Runs every check in order and stops at the first failure.
///
/// # Errors
/// Returns the first [`Failure`].
pub async fn run_all(store: &dyn BlobStore) -> Outcome {
    put_get_roundtrip(store).await?;
    missing_blobs(store).await?;
    rename_never_clobbers(store).await?;
    list_is_recursive_and_scoped(store).await
}

/// Content survives a round trip and `head` agrees with `get`.
///
/// # Errors
/// Returns a [`Failure`] describing the mismatch.
pub async fn put_get_roundtrip(store: &dyn BlobStore) -> Outcome {
    const C: &str = "put_get_roundtrip";
    let cases: [(&str, &[u8]); 3] = [
        ("empty.md", b""),
        ("deep/er/Notes – ü.md", b"# Hi\n"),
        ("bin", &[0, 255, 10, 13]),
    ];
    for (rel, body) in cases {
        let path = p(C, rel)?;
        let put = store
            .put(&path, Bytes::copy_from_slice(body), Precondition::None)
            .await
            .map_err(fail(C))?;
        let got = store.get(&path).await.map_err(fail(C))?;
        let head = store.head(&path).await.map_err(fail(C))?;
        ensure!(C, got.bytes.as_ref() == body, "{rel}: content differs");
        ensure!(
            C,
            put.size == body.len() as u64 && got.meta.size == put.size,
            "{rel}: size"
        );
        ensure!(
            C,
            got.meta.etag == put.etag && head.etag == put.etag,
            "{rel}: etags disagree"
        );
        ensure!(C, head.path == path, "{rel}: head path");
    }
    let path = p(C, "deep/er/Notes – ü.md")?;
    let before = store.head(&path).await.map_err(fail(C))?;
    let after = store
        .put(&path, Bytes::from_static(b"# Bye\n"), Precondition::None)
        .await
        .map_err(fail(C))?;
    ensure!(
        C,
        before.etag != after.etag,
        "etag unchanged after new content"
    );
    let got = store.get(&path).await.map_err(fail(C))?;
    ensure!(C, got.bytes.as_ref() == b"# Bye\n", "overwrite not visible");
    Ok(())
}

/// Every operation on a missing blob reports `NotFound`.
///
/// # Errors
/// Returns a [`Failure`] naming the operation that misbehaved.
pub async fn missing_blobs(store: &dyn BlobStore) -> Outcome {
    const C: &str = "missing_blobs";
    let (ghost, other) = (p(C, "ghost.md")?, p(C, "other.md")?);
    ensure!(
        C,
        matches!(store.get(&ghost).await, Err(StoreError::NotFound(_))),
        "get"
    );
    ensure!(
        C,
        matches!(store.head(&ghost).await, Err(StoreError::NotFound(_))),
        "head"
    );
    ensure!(
        C,
        matches!(
            store.delete(&ghost, Precondition::None).await,
            Err(StoreError::NotFound(_))
        ),
        "delete"
    );
    ensure!(
        C,
        matches!(
            store.rename(&ghost, &other).await,
            Err(StoreError::NotFound(_))
        ),
        "rename"
    );
    Ok(())
}
/// Rename moves content and refuses to replace an existing blob.
///
/// # Errors
/// Returns a [`Failure`] if a rename clobbers or loses content.
pub async fn rename_never_clobbers(store: &dyn BlobStore) -> Outcome {
    const C: &str = "rename_never_clobbers";
    let (a, b, c) = (p(C, "a.md")?, p(C, "sub/b.md")?, p(C, "c.md")?);
    let _ = store
        .put(&a, Bytes::from_static(b"A"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let _ = store
        .put(&c, Bytes::from_static(b"C"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let moved = store.rename(&a, &b).await.map_err(fail(C))?;
    ensure!(
        C,
        moved.path == b && moved.size == 1,
        "rename metadata: {moved:?}"
    );
    ensure!(
        C,
        matches!(store.head(&a).await, Err(StoreError::NotFound(_))),
        "source remains"
    );
    let clash = store.rename(&b, &c).await;
    ensure!(
        C,
        matches!(clash, Err(StoreError::AlreadyExists(_))),
        "clobber: {clash:?}"
    );
    ensure!(
        C,
        store.get(&b).await.map_err(fail(C))?.bytes.as_ref() == b"A",
        "source changed"
    );
    ensure!(
        C,
        store.get(&c).await.map_err(fail(C))?.bytes.as_ref() == b"C",
        "target changed"
    );
    Ok(())
}

/// Listing is recursive, scoped to the directory, and complete.
///
/// # Errors
/// Returns a [`Failure`] with the unexpected listing.
pub async fn list_is_recursive_and_scoped(store: &dyn BlobStore) -> Outcome {
    const C: &str = "list_is_recursive_and_scoped";
    for rel in ["d/one.md", "d/x/two.md", "d/x/y/three.md", "dx/outside.md"] {
        let _ = store
            .put(&p(C, rel)?, Bytes::from_static(b"."), Precondition::None)
            .await
            .map_err(fail(C))?;
    }
    let dir = p(C, "d")?;
    let mut got: Vec<String> = store
        .list(Some(&dir))
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await
        .map_err(fail(C))?;
    got.sort();
    let want: Vec<String> = ["d/one.md", "d/x/two.md", "d/x/y/three.md"]
        .iter()
        .map(|r| format!("conformance/{C}/{r}"))
        .collect();
    ensure!(C, got == want, "listing {got:?}");
    let empty = store.list(Some(&p(C, "nothing")?)).count().await;
    ensure!(C, empty == 0, "missing dir listed {empty} entries");
    Ok(())
}
