// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps each replica's in-memory caches in step with what the other
//! replicas write (ADR 0038).
//!
//! Every write through [`AnnouncingStore`] appends the file's path to the
//! shared change feed in the index database, and the drive appends each
//! space it writes. [`follow`] reads the feed four times a second and,
//! for changes another replica made, reloads only what they touched:
//!
//! - the collection folders whose files changed;
//! - the book list, when a `book.yaml` or `SUMMARY.md` changed;
//! - the path-to-space list behind permission checks, when a space did.
//!
//! A replica that falls behind the pruned feed reloads everything. The
//! space list is still reloaded every 30 s as a backstop, in case an
//! announcement was lost to a database error after its write succeeded.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use vellum_books_service::BooksService;
use vellum_collections::CollectionsService;
use vellum_collections::service::collection_dir;
use vellum_contracts::BoxFuture;
use vellum_db::feed::{Change, Feed, PATH, SPACES};
use vellum_store::layout::Kind;
use vellum_store::{
    Blob, BlobEntry, BlobMeta, BlobPath, BlobStore, Precondition, StoreResult,
};

/// How often a replica reads the feed. The brief wants a change on one
/// replica visible on the others within a second.
pub const POLL: Duration = Duration::from_millis(250);

/// Most changes read per poll; a longer backlog is read over several.
const PAGE: usize = 2048;

/// How many positions are kept when the feed is pruned (by one replica,
/// see [`crate::cluster`]). At 1,000 writes a second that is about two
/// minutes of history, far more than a replica falls behind between polls.
pub const KEEP: u64 = 100_000;

/// A name for this process that no other replica shares: the host (the
/// pod name in Kubernetes), the process ID and the start time.
#[must_use]
pub fn origin(var: impl Fn(&str) -> Option<String>) -> String {
    let host = var("VELLUM_NODE_NAME")
        .or_else(|| var("HOSTNAME"))
        .unwrap_or_else(|| "vellum".to_owned());
    format!(
        "{host}:{}:{}",
        std::process::id(),
        vellum_db::cluster::now_ms()
    )
}

/// Whether other replicas may cache something about `path`. Vellum's own
/// state (Work's journal, trash) and editor scratch files are not.
fn announced(path: &BlobPath) -> bool {
    !matches!(Kind::of(path), Kind::System | Kind::Scratch)
}

/// A [`BlobStore`] that announces every successful write in the feed.
#[derive(Debug)]
pub struct AnnouncingStore {
    inner: Arc<dyn BlobStore>,
    feed: Feed,
    origin: Arc<str>,
}

impl AnnouncingStore {
    /// Wraps `inner`, announcing as `origin`.
    #[must_use]
    pub fn new(inner: Arc<dyn BlobStore>, feed: Feed, origin: &str) -> Self {
        Self {
            inner,
            feed,
            origin: Arc::from(origin),
        }
    }

    async fn announce(&self, paths: &[&BlobPath]) {
        let items: Vec<(&str, &str)> = paths
            .iter()
            .filter(|p| announced(p))
            .map(|p| (PATH, p.as_str()))
            .collect();
        if items.is_empty() {
            return;
        }
        // The write already happened; other replicas catch up at their
        // next full reload if this is lost.
        if let Err(error) = self.feed.publish(&self.origin, &items).await {
            tracing::warn!(%error, "a write was not announced to the other \
                replicas");
        }
    }
}

#[async_trait]
impl BlobStore for AnnouncingStore {
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        self.inner.get(path).await
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        self.inner.head(path).await
    }

    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        let meta = self.inner.put(path, bytes, condition).await?;
        self.announce(&[path]).await;
        Ok(meta)
    }

    async fn delete(
        &self,
        path: &BlobPath,
        condition: Precondition,
    ) -> StoreResult<()> {
        self.inner.delete(path, condition).await?;
        self.announce(&[path]).await;
        Ok(())
    }

    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        let meta = self.inner.rename(from, to).await?;
        self.announce(&[from, to]).await;
        Ok(meta)
    }

    fn list(
        &self,
        dir: Option<&BlobPath>,
    ) -> BoxStream<'static, StoreResult<BlobEntry>> {
        self.inner.list(dir)
    }
}

/// What a batch of other replicas' changes touched.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Touched {
    /// Collection folders to re-read.
    pub collections: BTreeSet<BlobPath>,
    /// A book manifest changed: forget where books are.
    pub books: bool,
    /// A space changed: reload the space list.
    pub spaces: bool,
}

/// Sorts `changes` not made by `me` into what they touched.
#[must_use]
pub fn touched(changes: &[Change], me: &str) -> Touched {
    let mut out = Touched::default();
    for change in changes.iter().filter(|c| c.origin != me) {
        match change.topic.as_str() {
            SPACES => out.spaces = true,
            PATH => {
                let Ok(path) = BlobPath::new(&change.subject) else {
                    continue;
                };
                if let Some(dir) = collection_dir(&path) {
                    let _ = out.collections.insert(dir);
                }
                if matches!(Kind::of(&path), Kind::BookManifest { .. }) {
                    out.books = true;
                }
            }
            // A topic from a newer replica during a rolling deploy.
            _ => {}
        }
    }
    out
}

/// The caches [`follow`] keeps fresh.
pub struct Caches {
    /// Collections, reloaded per folder.
    pub collections: Arc<CollectionsService>,
    /// The book list.
    pub books: Arc<BooksService>,
    /// Reloads the path-to-space list.
    pub spaces: Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>,
    /// MCP listeners, told about every change, this replica's included.
    pub events: Option<crate::mcp_serve::Events>,
}

impl Caches {
    async fn apply(&self, touched: Touched) {
        if touched.spaces {
            (self.spaces)().await;
        }
        if touched.books {
            self.books.invalidate().await;
        }
        self.collections.refresh(&touched.collections).await;
    }

    async fn reset(&self) {
        if let Some(events) = &self.events {
            events.reset();
        }
        (self.spaces)().await;
        self.books.invalidate().await;
        self.collections.invalidate().await;
    }
}

/// Follows `feed` from `cursor` for as long as the server runs, applying
/// other replicas' changes to `caches`.
pub fn follow(feed: Feed, me: String, mut cursor: i64, caches: Caches) {
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            // Drain a backlog without waiting a tick between pages.
            loop {
                match feed.read(cursor, PAGE).await {
                    Ok(page) if page.reset => {
                        tracing::info!(
                            "missed changes from other replicas; reloading \
                                caches"
                        );
                        caches.reset().await;
                        cursor = page.cursor;
                        break;
                    }
                    Ok(page) => {
                        let full = page.changes.len() >= PAGE;
                        if let Some(events) = &caches.events {
                            events.changed(&page.changes);
                        }
                        caches.apply(touched(&page.changes, &me)).await;
                        cursor = page.cursor;
                        if !full {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cannot read the change feed");
                        break;
                    }
                }
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(origin: &str, topic: &str, subject: &str) -> Change {
        Change {
            seq: 1,
            topic: topic.to_owned(),
            subject: subject.to_owned(),
            origin: origin.to_owned(),
        }
    }

    fn path(p: &str) -> BlobPath {
        BlobPath::new(p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    #[test]
    fn own_changes_are_skipped_and_the_rest_sorted() {
        let changes = [
            change("me", PATH, "spaces/eng/tasks.collection/rows/a.md"),
            change("b", PATH, "spaces/eng/bugs.collection/rows/fix.md"),
            change("b", PATH, "spaces/eng/bugs.collection/schema.yaml"),
            change("b", PATH, "spaces/ops/hosts.collection/rows/x.md"),
            change("b", PATH, "spaces/eng/notes.md"),
            change("b", "future-topic", "whatever"),
        ];
        let t = touched(&changes, "me");
        assert_eq!(
            t.collections,
            BTreeSet::from([
                path("spaces/eng/bugs.collection"),
                path("spaces/ops/hosts.collection")
            ])
        );
        assert!(!t.books && !t.spaces);
    }

    #[test]
    fn books_and_spaces_are_flagged() {
        let t = touched(
            &[
                change("b", PATH, "spaces/eng/guide.book/SUMMARY.md"),
                change("b", SPACES, "spaces/new"),
            ],
            "me",
        );
        assert!(t.books && t.spaces && t.collections.is_empty());
        // A chapter edit does not move any book.
        let t = touched(
            &[change("b", PATH, "spaces/eng/guide.book/intro.md")],
            "me",
        );
        assert_eq!(t, Touched::default());
    }

    #[test]
    fn only_user_files_are_announced() {
        assert!(announced(&path("spaces/eng/a.md")));
        assert!(!announced(&path(".vellum/work/journal.json")));
        assert!(!announced(&path("spaces/eng/.a.md.swp")));
    }

    #[test]
    fn origins_differ_by_host() {
        let a = origin(|k| (k == "HOSTNAME").then(|| "pod-a".to_owned()));
        let b =
            origin(|k| (k == "VELLUM_NODE_NAME").then(|| "pod-b".to_owned()));
        assert!(a.starts_with("pod-a:") && b.starts_with("pod-b:"));
    }

    /// Two replicas over one index and one set of files (the #438 check):
    /// a collection row and a space written on A reach B's caches within
    /// a second, through the feed alone.
    #[tokio::test]
    async fn a_write_on_one_replica_reaches_another_within_a_second() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use vellum_authz::{Authz, MemoryStore};
        use vellum_db::{Index, IndexConfig};
        use vellum_drive::sql::SqlIndex;
        use vellum_drive::{DriveIndex as _, Space, SpaceKind, SpaceSettings};
        use vellum_kernel::SpaceId;
        use vellum_store::LocalStore;

        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let data = dir.path().join("data");
        let index = Index::open(&IndexConfig::sqlite(
            dir.path().join("index.sqlite"),
            None,
        ))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        let feed = Feed::new(index.clone());
        let files: Arc<dyn BlobStore> = Arc::new(
            LocalStore::open(&data)
                .await
                .unwrap_or_else(|e| panic!("{e}")),
        );
        let replica = |name: &str| -> Arc<dyn BlobStore> {
            Arc::new(AnnouncingStore::new(
                Arc::clone(&files),
                feed.clone(),
                name,
            ))
        };
        let (a, b) = (replica("a"), replica("b"));

        let authz = Arc::new(Authz::new(MemoryStore::new()));
        let space_of: vellum_collections::service::SpaceOf =
            Arc::new(|_: &BlobPath| None);
        let collections = Arc::new(CollectionsService::new(
            Arc::clone(&b),
            Arc::new(vellum_collections::service::AuthzAccess::new(
                Arc::clone(&authz),
                Arc::clone(&space_of),
            )),
        ));
        let books = Arc::new(BooksService::new(
            Arc::clone(&b),
            Arc::new(vellum_books_service::AuthzAccess::new(authz, space_of)),
        ));
        assert!(
            collections
                .index_rows()
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .is_empty()
        );
        let reloads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&reloads);
        follow(
            feed.clone(),
            "b".to_owned(),
            feed.head().await.unwrap_or_else(|e| panic!("{e}")),
            Caches {
                collections: Arc::clone(&collections),
                books,
                spaces: Box::new(move || {
                    let _ = counter.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async {})
                }),
                events: None,
            },
        );

        // Replica A writes a collection; B already holds an empty one.
        let dir_path = "spaces/eng/bugs.collection";
        let row = "01JBQ8Y4ZK6V3W2N5R7T9X1C41";
        let put = |p: String, text: String| {
            let a = Arc::clone(&a);
            async move {
                let _ = a
                    .put(&path(&p), Bytes::from(text), Precondition::None)
                    .await
                    .unwrap_or_else(|e| panic!("{e}"));
            }
        };
        put(
            format!("{dir_path}/schema.yaml"),
            "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C30\ntitle: Bugs\n\
                properties:\n  - {key: t, name: Bug, type: title}\n"
                .to_owned(),
        )
        .await;
        put(
            format!("{dir_path}/rows/crash.md"),
            format!("---\nid: {row}\nt: Crash on save\n---\n"),
        )
        .await;
        let started = tokio::time::Instant::now();
        loop {
            let rows = collections
                .index_rows()
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            if rows.iter().any(|r| r.to_string() == row) {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "row not seen on B"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Replica A creates a space; B reloads its space list.
        let before = reloads.load(Ordering::SeqCst);
        SqlIndex::new(index)
            .put_space(&Space {
                id: SpaceId::generate(),
                kind: SpaceKind::SharedDrive,
                slug: "ops".to_owned(),
                name: "Ops".to_owned(),
                owner: None,
                settings: SpaceSettings::default(),
                root: path("drives/shared/ops"),
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let started = tokio::time::Instant::now();
        while reloads.load(Ordering::SeqCst) == before {
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "space not seen on B"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
