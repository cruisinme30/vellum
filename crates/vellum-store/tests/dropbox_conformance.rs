// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Dropbox adapter passes the storage conformance suite against an
//! in-process fake of the Dropbox API, bare and wrapped in the metrics
//! decorator, and handles the Dropbox specifics: ASCII-only argument
//! headers, token refresh on `401`, paged listings and upload sessions.

#[path = "fakes/dropbox.rs"]
mod fake;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use bytes::Bytes;
use fake::FakeDropbox;
use futures::{StreamExt, TryStreamExt};
use vellum_store::{
    BlobPath, BlobStore, Metered, Precondition, StoreError, StoreResult, conformance,
    remote::{DropboxConfig, DropboxStore, Secret, StaticToken, TokenSource},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const TOKEN: &str = "sl.test-token";
const ROOT: &str = "/Apps/Vellum";

fn config(fake: &FakeDropbox, tokens: Arc<dyn TokenSource>) -> DropboxConfig {
    DropboxConfig::new(ROOT, tokens).with_urls(&fake.url, &fake.url)
}

async fn store() -> Result<(FakeDropbox, DropboxStore), Box<dyn std::error::Error>> {
    let fake = FakeDropbox::start(TOKEN).await?;
    let store = DropboxStore::new(config(&fake, Arc::new(StaticToken::new(TOKEN))))?;
    Ok((fake, store))
}

fn path(raw: &str) -> StoreResult<BlobPath> {
    BlobPath::new(raw)
}

#[tokio::test(flavor = "multi_thread")]
async fn dropbox_store_conforms() -> TestResult {
    let (_fake, store) = store().await?;
    conformance::run_all(&store).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn metered_dropbox_store_conforms() -> TestResult {
    let (_fake, store) = store().await?;
    conformance::run_all(&Metered::new(store)).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dropbox_store_conforms_through_upload_sessions() -> TestResult {
    let fake = FakeDropbox::start(TOKEN).await?;
    let mut config = config(&fake, Arc::new(StaticToken::new(TOKEN)));
    config.single_upload_limit = 1;
    config.upload_chunk = 2;
    conformance::run_all(&DropboxStore::new(config)?).await?;
    // Blobs of at most one byte still go up in one call.
    assert!(fake.count("files/upload_session/finish") > 10);
    assert!(fake.count("files/upload_session/append_v2") > 0);
    Ok(())
}

#[tokio::test]
async fn large_blobs_go_up_in_chunks() -> TestResult {
    let fake = FakeDropbox::start(TOKEN).await?;
    let mut config = config(&fake, Arc::new(StaticToken::new(TOKEN)));
    config.single_upload_limit = 4;
    config.upload_chunk = 3;
    let store = DropboxStore::new(config)?;
    let p = path("big.bin")?;
    let body = Bytes::from_static(b"0123456789");
    let put = store.put(&p, body.clone(), Precondition::IfAbsent).await?;
    assert_eq!(put.size, 10);
    assert_eq!(store.get(&p).await?.bytes, body);
    assert_eq!(fake.count("files/upload_session/start"), 1);
    assert_eq!(fake.count("files/upload_session/append_v2"), 2);
    assert_eq!(fake.count("files/upload_session/finish"), 1);
    let again = store.put(&p, body, Precondition::IfAbsent).await;
    assert!(
        matches!(again, Err(StoreError::AlreadyExists(_))),
        "{again:?}"
    );
    Ok(())
}

#[tokio::test]
async fn non_ascii_paths_are_escaped_in_the_arg_header() -> TestResult {
    let (fake, store) = store().await?;
    let p = path("Notes – ü.md")?;
    let _ = store
        .put(&p, Bytes::from_static(b"# Hi\n"), Precondition::None)
        .await?;
    let got = store.get(&p).await?;
    assert_eq!(got.bytes.as_ref(), b"# Hi\n");
    assert_eq!(got.meta.path, p);
    assert_eq!(
        fake.display("/Apps/Vellum/Notes – ü.md").as_deref(),
        Some("/Apps/Vellum/Notes – ü.md")
    );
    let args: Vec<Vec<u8>> = fake.calls().into_iter().filter_map(|c| c.api_arg).collect();
    assert_eq!(args.len(), 2, "upload and download");
    for arg in &args {
        assert!(arg.is_ascii(), "{}", String::from_utf8_lossy(arg));
        let text = String::from_utf8_lossy(arg);
        assert!(text.contains(r"Notes \u2013 \u00fc.md"), "{text}");
    }
    let listed: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    assert_eq!(listed, ["Notes – ü.md"]);
    Ok(())
}

/// Hands out a stale token until invalidated, then the good one.
#[derive(Debug, Default)]
struct Rotating {
    invalidations: AtomicUsize,
    good: Option<&'static str>,
}

#[async_trait]
impl TokenSource for Rotating {
    async fn token(&self) -> StoreResult<Secret> {
        let fresh = self.invalidations.load(Ordering::SeqCst) > 0;
        Ok(Secret::new(match self.good {
            Some(good) if fresh => good,
            _ => "sl.expired",
        }))
    }

    async fn invalidate(&self) {
        let _ = self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_rejected_token_is_invalidated_and_retried_once() -> TestResult {
    let fake = FakeDropbox::start(TOKEN).await?;
    let tokens = Arc::new(Rotating {
        good: Some(TOKEN),
        ..Rotating::default()
    });
    let store = DropboxStore::new(config(&fake, tokens.clone()))?;
    let p = path("a.md")?;
    let _ = store
        .put(&p, Bytes::from_static(b"x"), Precondition::None)
        .await?;
    assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 1);
    let statuses: Vec<u16> = fake.calls().iter().map(|c| c.status).collect();
    assert_eq!(statuses, [401, 200]);
    Ok(())
}

#[tokio::test]
async fn a_token_rejected_twice_is_a_backend_error() -> TestResult {
    let fake = FakeDropbox::start(TOKEN).await?;
    let tokens = Arc::new(Rotating::default());
    let store = DropboxStore::new(config(&fake, tokens.clone()))?;
    let got = store.head(&path("a.md")?).await;
    assert!(
        matches!(&got, Err(StoreError::Backend(m)) if m.contains("401") && !m.contains("sl.expired")),
        "{got:?}"
    );
    assert_eq!(tokens.invalidations.load(Ordering::SeqCst), 2);
    assert_eq!(fake.count("files/get_metadata"), 2);
    Ok(())
}

#[tokio::test]
async fn listings_follow_the_cursor_across_pages() -> TestResult {
    let (fake, store) = store().await?;
    fake.set_page_limit(2);
    let mut want = Vec::new();
    for rel in ["a.md", "b/c.md", "b/d/e.md", "f.md", "g/h.md"] {
        let _ = store
            .put(&path(rel)?, Bytes::from_static(b"."), Precondition::None)
            .await?;
        want.push(rel.to_owned());
    }
    let mut got: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    got.sort();
    assert_eq!(got, want);
    // Five files and four folders (root, b, b/d, g) in pages of two.
    assert_eq!(fake.count("files/list_folder"), 1);
    assert_eq!(fake.count("files/list_folder/continue"), 4);
    Ok(())
}

#[tokio::test]
async fn listings_hide_temp_files_and_invalid_names() -> TestResult {
    let (fake, store) = store().await?;
    fake.insert("/Apps/Vellum/ok.md", b"1");
    fake.insert("/Apps/Vellum/.vellum-tmp-1-0", b"partial");
    fake.insert("/Apps/Vellum/dir/.vellum-tmp-2-0", b"partial");
    fake.insert("/Apps/Vellum/back\\slash.md", b"3");
    fake.insert("/Apps/Other/outside.md", b"4");
    let got: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    assert_eq!(got, ["ok.md"]);
    let none = store.list(Some(&path("missing")?)).count().await;
    assert_eq!(none, 0);
    let file: Vec<_> = store.list(Some(&path("ok.md")?)).try_collect().await?;
    assert_eq!(file.len(), 1);
    Ok(())
}

#[tokio::test]
async fn paths_are_case_insensitive() -> TestResult {
    let (_fake, store) = store().await?;
    let _ = store
        .put(
            &path("Notes/A.md")?,
            Bytes::from_static(b"a"),
            Precondition::None,
        )
        .await?;
    let head = store.head(&path("notes/a.md")?).await?;
    assert_eq!(head.size, 1);
    let clash = store
        .put(
            &path("NOTES/a.MD")?,
            Bytes::from_static(b"b"),
            Precondition::IfAbsent,
        )
        .await;
    assert!(
        matches!(clash, Err(StoreError::AlreadyExists(_))),
        "{clash:?}"
    );
    let listed: Vec<String> = store
        .list(None)
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    assert_eq!(listed, ["Notes/A.md"]);
    Ok(())
}

#[tokio::test]
async fn folders_are_never_deleted_or_moved_as_blobs() -> TestResult {
    let (_fake, store) = store().await?;
    let inside = path("dir/a.md")?;
    let _ = store
        .put(&inside, Bytes::from_static(b"a"), Precondition::None)
        .await?;
    let dir = path("dir")?;
    let deleted = store.delete(&dir, Precondition::None).await;
    assert!(
        matches!(deleted, Err(StoreError::NotFound(_))),
        "{deleted:?}"
    );
    let moved = store.rename(&dir, &path("other")?).await;
    assert!(matches!(moved, Err(StoreError::NotFound(_))), "{moved:?}");
    assert!(matches!(
        store.get(&dir).await,
        Err(StoreError::NotFound(_))
    ));
    assert_eq!(store.get(&inside).await?.bytes.as_ref(), b"a");
    Ok(())
}

#[tokio::test]
async fn delete_if_absent_fails_either_way() -> TestResult {
    let (_fake, store) = store().await?;
    let p = path("a.md")?;
    let missing = store.delete(&p, Precondition::IfAbsent).await;
    assert!(
        matches!(missing, Err(StoreError::NotFound(_))),
        "{missing:?}"
    );
    let _ = store
        .put(&p, Bytes::from_static(b"a"), Precondition::None)
        .await?;
    let present = store.delete(&p, Precondition::IfAbsent).await;
    assert!(
        matches!(present, Err(StoreError::AlreadyExists(_))),
        "{present:?}"
    );
    assert!(store.head(&p).await.is_ok());
    Ok(())
}

#[test]
fn roots_must_be_absolute() {
    let tokens: Arc<dyn TokenSource> = Arc::new(StaticToken::new(TOKEN));
    let bad = DropboxStore::new(DropboxConfig::new("Apps/Vellum", tokens));
    assert!(
        matches!(bad, Err(StoreError::InvalidPath { .. })),
        "{bad:?}"
    );
}
