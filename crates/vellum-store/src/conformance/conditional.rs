// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Checks for conditional writes, the guarantee that keeps Vellum from
//! ever falling back to last-write-wins.

use bytes::Bytes;
use futures::future::join_all;

use super::{Failure, Outcome, fail, p};
use crate::{BlobStore, Precondition, StoreError};

/// `IfAbsent` creates once and never overwrites.
///
/// # Errors
/// Returns a [`Failure`] if a second create succeeds or changes content.
pub async fn if_absent(store: &dyn BlobStore) -> Outcome {
    const C: &str = "if_absent";
    let path = p(C, "a.md")?;
    let _ = store
        .put(&path, Bytes::from_static(b"first"), Precondition::IfAbsent)
        .await
        .map_err(fail(C))?;
    let again = store
        .put(&path, Bytes::from_static(b"second"), Precondition::IfAbsent)
        .await;
    ensure!(
        C,
        matches!(again, Err(StoreError::AlreadyExists(_))),
        "second create: {again:?}"
    );
    let got = store.get(&path).await.map_err(fail(C))?;
    ensure!(C, got.bytes.as_ref() == b"first", "content was overwritten");
    Ok(())
}

/// `IfMatch` writes only over the expected version and reports the current one.
///
/// # Errors
/// Returns a [`Failure`] if a stale write lands.
pub async fn if_match(store: &dyn BlobStore) -> Outcome {
    const C: &str = "if_match";
    let path = p(C, "a.md")?;
    let v1 = store
        .put(&path, Bytes::from_static(b"v1"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let v2 = store
        .put(
            &path,
            Bytes::from_static(b"v2"),
            Precondition::IfMatch(v1.etag.clone()),
        )
        .await
        .map_err(fail(C))?;
    let stale = store
        .put(
            &path,
            Bytes::from_static(b"v3"),
            Precondition::IfMatch(v1.etag),
        )
        .await;
    ensure!(
        C,
        matches!(
            &stale,
            Err(
                StoreError::PreconditionFailed { current: Some(e), .. }
            ) if *e == v2.etag
        ),
        "stale write: {stale:?}"
    );
    ensure!(
        C,
        store.get(&path).await.map_err(fail(C))?.bytes.as_ref() == b"v2",
        "stale write landed"
    );
    let gone = store
        .put(
            &p(C, "none.md")?,
            Bytes::new(),
            Precondition::IfMatch(v2.etag),
        )
        .await;
    ensure!(
        C,
        matches!(
            gone,
            Err(StoreError::PreconditionFailed { current: None, .. })
        ),
        "missing: {gone:?}"
    );
    Ok(())
}

/// Deletes honour `IfMatch`.
///
/// # Errors
/// Returns a [`Failure`] if a stale delete removes the blob.
pub async fn conditional_delete(store: &dyn BlobStore) -> Outcome {
    const C: &str = "conditional_delete";
    let path = p(C, "a.md")?;
    let v1 = store
        .put(&path, Bytes::from_static(b"v1"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let v2 = store
        .put(&path, Bytes::from_static(b"v2"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let stale = store.delete(&path, Precondition::IfMatch(v1.etag)).await;
    ensure!(
        C,
        matches!(stale, Err(StoreError::PreconditionFailed { .. })),
        "stale delete: {stale:?}"
    );
    store
        .delete(&path, Precondition::IfMatch(v2.etag))
        .await
        .map_err(fail(C))?;
    ensure!(
        C,
        matches!(store.head(&path).await, Err(StoreError::NotFound(_))),
        "still present"
    );
    Ok(())
}

/// Of many writers racing from the same version, exactly one wins.
///
/// # Errors
/// Returns a [`Failure`] if more or fewer than one write lands.
pub async fn racing_writers_one_wins(store: &dyn BlobStore) -> Outcome {
    const C: &str = "racing_writers_one_wins";
    let path = p(C, "a.md")?;
    let base = store
        .put(&path, Bytes::from_static(b"base"), Precondition::None)
        .await
        .map_err(fail(C))?;
    let writes = (0..8_u8).map(|i| {
        store.put(
            &path,
            Bytes::from(vec![b'w', i]),
            Precondition::IfMatch(base.etag.clone()),
        )
    });
    let results = join_all(writes).await;
    let wins = results.iter().filter(|r| r.is_ok()).count();
    let lost = results
        .iter()
        .filter(|r| matches!(r, Err(StoreError::PreconditionFailed { .. })))
        .count();
    ensure!(
        C,
        wins == 1 && lost == 7,
        "{wins} won, {lost} lost the race"
    );
    Ok(())
}
