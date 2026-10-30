// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `OneDrive` adapter passes the storage conformance suite against an
//! in-process fake of Microsoft Graph, bare and wrapped in the metrics
//! decorator, and handles upload sessions, paged listings and rejected
//! tokens.

#![cfg(feature = "onedrive")]

use crate::fakes::onedrive as fake;

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use async_trait::async_trait;
use bytes::Bytes;
use fake::{FakeGraph, Limits, ROOT};
use futures::TryStreamExt;
use vellum_store::{
    BlobPath, BlobStore, Metered, Precondition, StoreError, StoreResult, conformance,
    remote::{Secret, StaticToken, TokenSource, onedrive::OneDriveStore},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn store(fake: &FakeGraph) -> StoreResult<OneDriveStore> {
    OneDriveStore::new(fake.config(Arc::new(StaticToken::new("t1"))))
}

fn assert_no_leaks(fake: &FakeGraph) {
    let counters = fake.counters();
    assert_eq!(
        counters.leaked_auth, 0,
        "bearer token sent to a pre-authenticated URL"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn onedrive_store_conforms() -> TestResult {
    let fake = FakeGraph::start(Limits::default()).await?;
    conformance::run_all(&store(&fake)?).await?;
    assert_no_leaks(&fake);
    assert!(fake.counters().next_pages > 0, "listing never paged");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn metered_onedrive_store_conforms() -> TestResult {
    let fake = FakeGraph::start(Limits::default()).await?;
    conformance::run_all(&Metered::new(store(&fake)?)).await?;
    assert_no_leaks(&fake);
    Ok(())
}

/// With a tiny simple-upload limit, nearly every write in the suite goes
/// through an upload session, including the racing writers.
///
/// How many sessions are *created* depends on timing: a racing writer
/// whose session request arrives after the winner committed is refused at
/// creation and never opens one. Committed sessions do not: the suite's
/// sequential writes over one byte commit 9, and exactly one racer wins.
#[tokio::test(flavor = "multi_thread")]
async fn onedrive_store_conforms_through_upload_sessions() -> TestResult {
    let limits = Limits {
        simple_upload_max: 1,
        chunk_size: 2,
        ..Limits::default()
    };
    let fake = FakeGraph::start(limits).await?;
    conformance::run_all(&store(&fake)?).await?;
    assert_no_leaks(&fake);
    let counters = fake.counters();
    assert_eq!(counters.session_commits, 10, "{counters:?}");
    Ok(())
}

#[tokio::test]
async fn large_blobs_upload_in_chunks() -> TestResult {
    let limits = Limits {
        simple_upload_max: 64,
        chunk_size: 16,
        ..Limits::default()
    };
    let fake = FakeGraph::start(limits).await?;
    let store = store(&fake)?;
    let path = BlobPath::new("big/blob.bin")?;
    let body: Bytes = (0..100_u8).collect::<Vec<_>>().into();

    let put = store
        .put(&path, body.clone(), Precondition::IfAbsent)
        .await?;
    let counters = fake.counters();
    assert_eq!((counters.sessions, counters.chunks), (1, 7));
    assert_eq!(counters.simple_uploads, 0);
    assert_eq!(put.size, 100);

    let got = store.get(&path).await?;
    assert_eq!(got.bytes, body);
    assert_eq!(got.meta.etag, put.etag);

    let again = store.put(&path, body.clone(), Precondition::IfAbsent).await;
    assert!(
        matches!(again, Err(StoreError::AlreadyExists(_))),
        "{again:?}"
    );

    let next = store
        .put(
            &path,
            body.slice(1..),
            Precondition::IfMatch(put.etag.clone()),
        )
        .await?;
    let stale = store
        .put(&path, body.clone(), Precondition::IfMatch(put.etag))
        .await;
    assert!(
        matches!(&stale, Err(StoreError::PreconditionFailed { current: Some(e), .. }) if *e == next.etag),
        "{stale:?}"
    );
    assert_eq!(store.get(&path).await?.bytes, body.slice(1..));
    assert_no_leaks(&fake);
    Ok(())
}

#[tokio::test]
async fn listings_follow_next_links_and_hide_temp_files() -> TestResult {
    let fake = FakeGraph::start(Limits {
        page_size: 2,
        ..Limits::default()
    })
    .await?;
    let store = store(&fake)?;
    for i in 0..7 {
        let path = BlobPath::new(format!("notes/n{i}.md"))?;
        let _ = store
            .put(&path, Bytes::from_static(b"x"), Precondition::None)
            .await?;
    }
    fake.insert(&format!("{ROOT}/notes/.vellum-tmp-1-0"), b"partial");
    fake.insert(&format!("{ROOT}/notes/back\\slash.md"), b"invalid");
    fake.insert(
        &format!("{ROOT}/notes/.vellum-tmp-dir/hidden.md"),
        b"hidden",
    );
    fake.insert("outside-root.md", b"not ours");

    let mut listed: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    listed.sort();
    let want: Vec<String> = (0..7).map(|i| format!("notes/n{i}.md")).collect();
    assert_eq!(listed, want);
    assert!(fake.counters().next_pages >= 3, "{:?}", fake.counters());

    let one = BlobPath::new("notes/n3.md")?;
    let listed: Vec<_> = store.list(Some(&one)).try_collect().await?;
    assert_eq!(listed.len(), 1, "listing a file yields the file");
    Ok(())
}

#[tokio::test]
async fn missing_store_root_lists_nothing() -> TestResult {
    let fake = FakeGraph::start(Limits::default()).await?;
    let listed: Vec<_> = store(&fake)?.list(None).try_collect().await?;
    assert!(listed.is_empty());
    Ok(())
}

/// Hands out `t0`, `t1`, ... and moves on to the next token each time it
/// is invalidated.
#[derive(Debug, Default)]
struct Rotating {
    invalidations: AtomicU32,
}

#[async_trait]
impl TokenSource for Rotating {
    async fn token(&self) -> StoreResult<Secret> {
        Ok(Secret::new(format!(
            "t{}",
            self.invalidations.load(Ordering::SeqCst)
        )))
    }

    async fn invalidate(&self) {
        let _ = self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn rejected_tokens_are_refreshed_once() -> TestResult {
    let fake = FakeGraph::start(Limits::default()).await?;
    let tokens = Arc::new(Rotating::default());
    let store = OneDriveStore::new(fake.config(Arc::clone(&tokens) as Arc<dyn TokenSource>))?;
    let path = BlobPath::new("a.md")?;

    // `t0` is stale: one 401, one invalidation, then `t1` works.
    let _ = store
        .put(&path, Bytes::from_static(b"a"), Precondition::None)
        .await?;
    assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
    assert_eq!(fake.counters().unauthorized, 1);
    let _ = store.get(&path).await?;
    assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);

    // The server stops accepting anything: one retry, then a backend error.
    fake.accept_token("revoked");
    let got = store.head(&path).await;
    assert!(matches!(got, Err(StoreError::Backend(_))), "{got:?}");
    assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 2);
    assert_eq!(fake.counters().unauthorized, 3);
    assert_no_leaks(&fake);
    Ok(())
}

#[tokio::test]
async fn delete_refuses_folders() -> TestResult {
    let fake = FakeGraph::start(Limits::default()).await?;
    let store = store(&fake)?;
    let file = BlobPath::new("dir/a.md")?;
    let _ = store
        .put(&file, Bytes::from_static(b"a"), Precondition::None)
        .await?;
    let dir = BlobPath::new("dir")?;
    let gone = store.delete(&dir, Precondition::None).await;
    assert!(matches!(gone, Err(StoreError::NotFound(_))), "{gone:?}");
    assert_eq!(store.get(&file).await?.bytes.as_ref(), b"a");
    Ok(())
}
