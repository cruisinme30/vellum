// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Google Drive adapter against an in-process fake Drive: the
//! conformance suite (best effort, bare and metered), plus the Drive
//! specifics: duplicate names, quoting, the path cache, resumable uploads,
//! Google-native files, authentication and throttling.

#[path = "fakes/gdrive.rs"]
mod fake;

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use axum::http::StatusCode;
use bytes::Bytes;
use fake::{FakeDrive, GOOD_TOKEN, ROOT_ID};
use futures::{TryStreamExt, future::join_all};
use serde_json::json;
use vellum_store::{
    BlobPath, BlobStore, Metered, Precondition, StoreError, StoreResult,
    conformance::{self, Guarantees},
    remote::{
        Secret, StaticToken, TokenSource,
        gdrive::{GoogleDriveConfig, GoogleDriveStore},
    },
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DOC_MIME: &str = "application/vnd.google-apps.document";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

fn token() -> Arc<dyn TokenSource> {
    Arc::new(StaticToken::new(GOOD_TOKEN))
}

async fn store_with(
    edit: impl FnOnce(&mut GoogleDriveConfig),
) -> StoreResult<(FakeDrive, GoogleDriveStore)> {
    let drive = FakeDrive::start()
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    let mut config = drive.config(token());
    edit(&mut config);
    let store = GoogleDriveStore::new(config)?;
    Ok((drive, store))
}

async fn store() -> StoreResult<(FakeDrive, GoogleDriveStore)> {
    store_with(|_| {}).await
}

/// The single id in `ids`.
fn only(ids: Vec<String>) -> Result<String, String> {
    match <[String; 1]>::try_from(ids) {
        Ok([id]) => Ok(id),
        Err(ids) => Err(format!("expected one id, got {ids:?}")),
    }
}

fn path(raw: &str) -> StoreResult<BlobPath> {
    BlobPath::new(raw)
}

async fn put(
    store: &GoogleDriveStore,
    raw: &str,
    body: &'static [u8],
) -> StoreResult<()> {
    let _ = store
        .put(&path(raw)?, Bytes::from_static(body), Precondition::None)
        .await?;
    Ok(())
}

async fn listing(
    store: &dyn BlobStore,
    dir: Option<&str>,
) -> StoreResult<Vec<String>> {
    let dir = dir.map(BlobPath::new).transpose()?;
    let mut got: Vec<String> = store
        .list(dir.as_ref())
        .map_ok(|e| e.path.to_string())
        .try_collect()
        .await?;
    got.sort();
    Ok(got)
}

#[tokio::test]
async fn gdrive_store_conforms() -> TestResult {
    let (_drive, store) = store().await?;
    conformance::run_with(&store, Guarantees::BestEffort).await?;
    Ok(())
}

#[tokio::test]
async fn metered_gdrive_store_conforms() -> TestResult {
    let (_drive, store) = store().await?;
    conformance::run_with(&Metered::new(store), Guarantees::BestEffort).await?;
    Ok(())
}

#[tokio::test]
async fn uncached_gdrive_store_conforms() -> TestResult {
    let (_drive, store) =
        store_with(|c| c.cache_ttl = std::time::Duration::ZERO).await?;
    conformance::run_with(&store, Guarantees::BestEffort).await?;
    Ok(())
}

#[tokio::test]
async fn duplicate_names_resolve_to_the_oldest() -> TestResult {
    let (drive, store) = store().await?;
    put(&store, "dup/a.md", b"mine").await?;
    let folder = only(drive.ids(ROOT_ID, "dup"))?;
    // Another client adds an older twin and a younger one.
    let _ = drive.insert(&folder, "a.md", "text/markdown", b"younger", None);
    let _ = drive.insert(&folder, "a.md", "text/markdown", b"oldest", Some(0));
    // Same creation time as the oldest but a larger id: loses the tie.
    let _ = drive.insert(&folder, "a.md", "text/markdown", b"tied", Some(0));

    // Another process resolves the path afresh and picks the oldest.
    let other = GoogleDriveStore::new(drive.config(token()))?;
    let a = path("dup/a.md")?;
    assert_eq!(other.get(&a).await?.bytes.as_ref(), b"oldest");
    assert_eq!(listing(&other, Some("dup")).await?, ["dup/a.md"]);

    // Twin folders: the older one wins, for paths and listings alike.
    let older = drive.insert(ROOT_ID, "twins", FOLDER_MIME, b"", Some(0));
    let younger = drive.insert(ROOT_ID, "twins", FOLDER_MIME, b"", None);
    let _ = drive.insert(&older, "kept.md", "text/markdown", b"k", None);
    let _ = drive.insert(&younger, "hidden.md", "text/markdown", b"h", None);
    assert_eq!(listing(&other, Some("twins")).await?, ["twins/kept.md"]);
    assert!(matches!(
        other.get(&path("twins/hidden.md")?).await,
        Err(StoreError::NotFound(_))
    ));
    // Writes go to the winning folder too.
    put(&other, "twins/new.md", b"n").await?;
    assert_eq!(drive.ids(&older, "new.md").len(), 1);
    Ok(())
}

#[tokio::test]
async fn names_with_quotes_and_keywords_round_trip() -> TestResult {
    let (drive, store) = store().await?;
    let names = [
        "it's.md",
        "o'neil/'quoted' and \"double\".md",
        "rock and roll/in parents.md",
        "trashed = true.md",
    ];
    for (i, name) in names.iter().enumerate() {
        let _ = store
            .put(
                &path(name)?,
                Bytes::from(format!("{i}")),
                Precondition::IfAbsent,
            )
            .await?;
    }
    let fresh = GoogleDriveStore::new(drive.config(token()))?;
    for (i, name) in names.iter().enumerate() {
        assert_eq!(fresh.get(&path(name)?).await?.bytes, format!("{i}"));
    }
    let mut want: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    want.sort();
    assert_eq!(listing(&fresh, None).await?, want);
    let moved = fresh
        .rename(&path("it's.md")?, &path("o'neil/it's.md")?)
        .await?;
    assert_eq!(moved.size, 1);
    assert_eq!(
        fresh.get(&path("o'neil/it's.md")?).await?.bytes.as_ref(),
        b"0"
    );
    Ok(())
}

#[tokio::test]
async fn path_cache_survives_rename_and_delete() -> TestResult {
    let (drive, store) = store().await?;
    let (a, b) = (path("notes/a.md")?, path("notes/b.md")?);
    put(&store, "notes/a.md", b"one").await?;
    let first = store.head(&a).await?;

    // Cached: a second head reads metadata by id, without searching.
    let lists = drive.list_calls();
    assert_eq!(store.head(&a).await?, first);
    assert_eq!(drive.list_calls(), lists, "head searched despite the cache");

    let moved = store.rename(&a, &b).await?;
    assert_eq!(moved.etag, first.etag, "rename changed the content version");
    assert!(matches!(store.head(&a).await, Err(StoreError::NotFound(_))));
    assert_eq!(store.get(&b).await?.bytes.as_ref(), b"one");

    put(&store, "notes/a.md", b"two").await?;
    assert_eq!(store.get(&a).await?.bytes.as_ref(), b"two");
    store.delete(&b, Precondition::None).await?;
    assert!(matches!(store.get(&b).await, Err(StoreError::NotFound(_))));
    put(&store, "notes/b.md", b"three").await?;
    assert_eq!(store.get(&b).await?.bytes.as_ref(), b"three");

    // Another client deletes `a`: the cached id is dropped, not trusted.
    let notes = drive.ids(ROOT_ID, "notes");
    let notes = notes.first().ok_or("no notes folder")?;
    let old_a = only(drive.ids(notes, "a.md"))?;
    drive.remove(&old_a);
    assert!(matches!(store.get(&a).await, Err(StoreError::NotFound(_))));
    // ... and recreates it under a new id.
    let _ = drive.insert(notes, "a.md", "text/markdown", b"outside", None);
    assert_eq!(store.get(&a).await?.bytes.as_ref(), b"outside");

    // Trashing counts as gone, for the file and for its folder.
    let b_id = only(drive.ids(notes, "b.md"))?;
    drive.trash(&b_id);
    assert!(matches!(store.head(&b).await, Err(StoreError::NotFound(_))));
    drive.trash(notes);
    assert!(matches!(store.head(&a).await, Err(StoreError::NotFound(_))));
    assert!(listing(&store, Some("notes")).await?.is_empty());
    // Writing there again makes a new folder rather than reviving the old.
    put(&store, "notes/a.md", b"four").await?;
    assert_eq!(store.get(&a).await?.bytes.as_ref(), b"four");
    assert_eq!(drive.ids(ROOT_ID, "notes").len(), 1);
    Ok(())
}

#[tokio::test]
async fn large_files_use_resumable_uploads() -> TestResult {
    let (drive, store) = store_with(|c| {
        c.resumable_threshold = 1024;
        c.resumable_chunk = 256 * 1024;
    })
    .await?;
    let big: Vec<u8> = (0..600 * 1024_u32).map(|i| (i % 251) as u8).collect();
    let p = path("big/file.bin")?;
    let created = store
        .put(&p, Bytes::from(big.clone()), Precondition::IfAbsent)
        .await?;
    assert_eq!(created.size, big.len() as u64);
    assert_eq!(drive.chunk_puts(), 3, "600 KiB in 256 KiB chunks");
    assert_eq!(store.get(&p).await?.bytes.as_ref(), big.as_slice());

    // Updates resume from whatever Drive acknowledged.
    drive.accept_partial_chunks(true);
    let mut changed = big.clone();
    changed.reverse();
    let updated = store
        .put(
            &p,
            Bytes::from(changed.clone()),
            Precondition::IfMatch(created.etag.clone()),
        )
        .await?;
    assert_ne!(updated.etag, created.etag);
    assert!(drive.chunk_puts() > 6, "partial chunks were resent");
    assert_eq!(store.get(&p).await?.bytes.as_ref(), changed.as_slice());

    // Small writes stay simple.
    let before = drive.chunk_puts();
    put(&store, "big/small.md", b"tiny").await?;
    assert_eq!(drive.chunk_puts(), before);
    Ok(())
}

#[tokio::test]
async fn google_native_files_are_skipped() -> TestResult {
    let (drive, store) = store().await?;
    put(&store, "docs/real.md", b"r").await?;
    let docs = drive.ids(ROOT_ID, "docs");
    let docs = docs.first().ok_or("no docs folder")?;
    let _ = drive.insert(docs, "Plan", DOC_MIME, b"", None);
    let _ = drive.insert(
        docs,
        "Link",
        "application/vnd.google-apps.shortcut",
        b"",
        None,
    );
    let _ =
        drive.insert(docs, ".vellum-tmp-7-1", "text/plain", b"partial", None);
    let _ = drive.insert(docs, "a/b", "text/plain", b"slash", None);

    assert_eq!(listing(&store, None).await?, ["docs/real.md"]);
    let plan = path("docs/Plan")?;
    let got = store.get(&plan).await;
    assert!(
        matches!(&got, Err(StoreError::Backend(msg)) if msg.contains("Google-na\
            tive")),
        "{got:?}"
    );
    assert!(matches!(
        store.head(&plan).await,
        Err(StoreError::Backend(_))
    ));
    let overwrite = store
        .put(&plan, Bytes::from_static(b"x"), Precondition::None)
        .await;
    assert!(
        matches!(overwrite, Err(StoreError::Backend(_))),
        "{overwrite:?}"
    );
    let create = store
        .put(&plan, Bytes::from_static(b"x"), Precondition::IfAbsent)
        .await;
    assert!(
        matches!(create, Err(StoreError::AlreadyExists(_))),
        "{create:?}"
    );
    assert!(matches!(
        store.delete(&plan, Precondition::None).await,
        Err(StoreError::Backend(_))
    ));
    assert_eq!(drive.ids(docs, "Plan").len(), 1, "native file was touched");
    Ok(())
}

#[tokio::test]
async fn if_absent_racers_converge_on_one_file() -> TestResult {
    let (drive, store) = store().await?;
    let p = path("race/new folder/a.md")?;
    let writes = (0..8_u8)
        .map(|i| store.put(&p, Bytes::from(vec![i]), Precondition::IfAbsent));
    let results = join_all(writes).await;
    let wins: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let lost = results
        .iter()
        .filter(|r| matches!(r, Err(StoreError::AlreadyExists(_))))
        .count();
    assert_eq!((wins.len(), lost), (1, 7), "{results:?}");
    let race = drive.ids(ROOT_ID, "race");
    assert_eq!(race.len(), 1, "duplicate folders");
    let race = race.first().ok_or("no race folder")?;
    let inner = drive.ids(race, "new folder");
    assert_eq!(inner.len(), 1, "duplicate folders");
    let inner = inner.first().ok_or("no inner folder")?;
    assert_eq!(
        drive.ids(inner, "a.md").len(),
        1,
        "duplicate files survived"
    );
    let winner = wins.first().ok_or("no winner")?;
    assert_eq!(store.head(&p).await?.etag, winner.etag);
    Ok(())
}

#[tokio::test]
async fn deletes_are_permanent_unless_configured() -> TestResult {
    let (drive, store) = store().await?;
    put(&store, "a.md", b"a").await?;
    let id = only(drive.ids(ROOT_ID, "a.md"))?;
    store.delete(&path("a.md")?, Precondition::None).await?;
    assert!(drive.file(&id).is_none());

    let trashing = GoogleDriveStore::new({
        let mut c = drive.config(token());
        c.trash_on_delete = true;
        c
    })?;
    put(&trashing, "b.md", b"b").await?;
    let id = only(drive.ids(ROOT_ID, "b.md"))?;
    trashing.delete(&path("b.md")?, Precondition::None).await?;
    assert!(drive.file(&id).is_some_and(|f| f.trashed));
    assert!(matches!(
        trashing.head(&path("b.md")?).await,
        Err(StoreError::NotFound(_))
    ));
    Ok(())
}

/// Hands out tokens in order, moving on when one is invalidated.
#[derive(Debug)]
struct Rotating {
    tokens: Mutex<VecDeque<String>>,
    invalidated: AtomicUsize,
}

#[async_trait]
impl TokenSource for Rotating {
    async fn token(&self) -> StoreResult<Secret> {
        let tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        let token = tokens.front().cloned().unwrap_or_default();
        drop(tokens);
        Ok(Secret::new(token))
    }

    async fn invalidate(&self) {
        let _ = self.invalidated.fetch_add(1, Ordering::SeqCst);
        let _ = self
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
    }
}

#[tokio::test]
async fn unauthorized_refreshes_the_token_once() -> TestResult {
    let drive = FakeDrive::start().await?;
    let rotating = Arc::new(Rotating {
        tokens: Mutex::new(["expired", GOOD_TOKEN].map(str::to_owned).into()),
        invalidated: AtomicUsize::new(0),
    });
    let store = GoogleDriveStore::new(drive.config(rotating.clone()))?;
    put(&store, "a.md", b"a").await?;
    assert_eq!(rotating.invalidated.load(Ordering::SeqCst), 1);

    let revoked = Arc::new(Rotating {
        tokens: Mutex::new(["bad", "worse"].map(str::to_owned).into()),
        invalidated: AtomicUsize::new(0),
    });
    let store = GoogleDriveStore::new(drive.config(revoked.clone()))?;
    let got = store.head(&path("a.md")?).await;
    assert!(
        matches!(
            &got,
            Err(
                StoreError::Backend(msg)
            ) if msg.contains("401") && !msg.contains("worse")
        ),
        "{got:?}"
    );
    assert_eq!(revoked.invalidated.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn rate_limits_are_retried_but_denials_are_not() -> TestResult {
    let (drive, store) = store().await?;
    let limited = json!({
        "error": {
            "code": 403,
            "errors": [{ "reason": "userRateLimitExceeded" }]
        }
    });
    drive.inject(StatusCode::FORBIDDEN, limited.clone());
    drive.inject(StatusCode::FORBIDDEN, limited);
    put(&store, "a.md", b"a").await?;
    assert_eq!(store.get(&path("a.md")?).await?.bytes.as_ref(), b"a");

    let denied = json!({
        "error": {
            "code": 403,
            "errors": [{ "reason": "insufficientFilePermissions" }]
        }
    });
    drive.inject(StatusCode::FORBIDDEN, denied);
    let got = store.get(&path("a.md")?).await;
    assert!(
        matches!(&got, Err(StoreError::Backend(msg)) if msg.contains("insuffici\
            entFilePermissions")),
        "{got:?}"
    );
    Ok(())
}

#[tokio::test]
async fn listing_pages_through_large_folders() -> TestResult {
    let (drive, store) = store().await?;
    for i in 0..7 {
        let _ = drive.insert(
            ROOT_ID,
            &format!("n{i}.md"),
            "text/markdown",
            b"x",
            None,
        );
    }
    let lists = drive.list_calls();
    assert_eq!(listing(&store, None).await?.len(), 7);
    assert_eq!(drive.list_calls() - lists, 4, "pages of two");
    assert_eq!(drive.len(), 8);
    let one = listing(&store, Some("n3.md")).await?;
    assert_eq!(one, ["n3.md"], "listing a file lists the file");
    Ok(())
}
