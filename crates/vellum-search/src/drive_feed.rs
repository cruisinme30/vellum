// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps the search index in step with the drive's saved pages.
//!
//! Two halves:
//!
//! - [`DriveSource`] is the [`DocumentSource`] the [`Indexer`] reads from.
//!   It looks a page up in the drive's own tables (`drive_nodes`), reads
//!   the file from the store and turns it into blocks with
//!   [`crate::markdown::blocks`].
//! - [`DriveFeed`] notices which pages changed and queues them. It sweeps
//!   `drive_nodes` on an interval and compares each page's fingerprint
//!   (place, path, title, size, modified time, trash state) with the last
//!   sweep. That catches every way a page changes: saves in the app, edits
//!   made outside it that the file watcher picks up, startup rescans,
//!   moves, renames, trash and purges, including pages removed with their
//!   folder. A page that is gone is queued too, and the indexer removes it.
//!
//! The drive emits no events of its own, and its rescans record no
//! activity, so a sweep is the one signal that covers them all.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::task::JoinHandle;
use uuid::Uuid;
use vellum_db::{Index, IndexPool, sqlx, traced};
use vellum_kernel::{DocId, ObjectId, SpaceId, Timestamp, UserId};
use vellum_store::{BlobPath, BlobStore, StoreError};

use crate::error::{Result, SearchError};
use crate::indexer::{DocumentSource, Indexer};
use crate::model::{DocKind, IndexDocument};

/// The deepest folder nesting walked when collecting a page's ancestors.
const MAX_DEPTH: i32 = 64;

const LIST: &str = "SELECT node_id FROM drive_nodes WHERE kind = 'page'";

const NODE: &str = "SELECT space_id, kind, path, title, modified_at, created_by, trash \
                    FROM drive_nodes WHERE node_id = $1";

const ANCESTORS: &str = "WITH RECURSIVE up (node_id, parent, kind, depth) AS ( \
         SELECT node_id, tree_parent_id, kind, 0 FROM drive_nodes WHERE node_id = $1 \
         UNION ALL \
         SELECT n.node_id, n.tree_parent_id, n.kind, up.depth + 1 \
         FROM drive_nodes n JOIN up ON n.node_id = up.parent WHERE up.depth < $2 \
     ) SELECT node_id, kind FROM up WHERE depth > 0 ORDER BY depth DESC";

const SWEEP: &str = "SELECT node_id, space_id, tree_parent_id, path, title, byte_size, \
                     modified_at, trash FROM drive_nodes WHERE kind = 'page'";

fn failed(op: &str, error: &sqlx::Error) -> SearchError {
    SearchError::Source(format!("{op}: {error}"))
}

fn unsupported() -> SearchError {
    SearchError::Source("unsupported index backend".into())
}

/// Runs one read on whichever database the index uses.
macro_rules! fetch {
    ($index:expr, $op:literal, $method:ident, $query:expr) => {
        match $index.pool() {
            IndexPool::Sqlite(pool) => traced("sqlite", $op, $query.$method(pool)).await,
            IndexPool::Postgres(pool) => traced("postgresql", $op, $query.$method(pool)).await,
            _ => return Err(unsupported()),
        }
        .map_err(|e| failed($op, &e))?
    };
}

const fn uuid(id: u128) -> Uuid {
    Uuid::from_u128(id)
}

/// Reads saved pages for the indexer, from the drive's tables and files.
#[derive(Clone)]
pub struct DriveSource {
    index: Arc<Index>,
    store: Arc<dyn BlobStore>,
}

impl std::fmt::Debug for DriveSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveSource")
            .field("store", &self.store.backend())
            .finish_non_exhaustive()
    }
}

type NodeRow = (
    Uuid,
    String,
    String,
    String,
    i64,
    Option<Uuid>,
    Option<String>,
);

impl DriveSource {
    /// A source over the index the drive writes and the store it reads
    /// pages from (the same store, so encrypted spaces read as plain text).
    #[must_use]
    pub fn new(index: Arc<Index>, store: Arc<dyn BlobStore>) -> Self {
        Self { index, store }
    }

    /// The node's containers, outermost first: folders and books.
    async fn ancestors(&self, node: Uuid) -> Result<Vec<(Uuid, String)>> {
        Ok(fetch!(
            self.index,
            "search.feed.ancestors",
            fetch_all,
            sqlx::query_as(ANCESTORS).bind(node).bind(MAX_DEPTH)
        ))
    }

    async fn text(&self, path: &str) -> Result<String> {
        let path = BlobPath::new(path).map_err(|e| SearchError::Source(e.to_string()))?;
        match self.store.get(&path).await {
            Ok(blob) => Ok(String::from_utf8_lossy(&blob.bytes).into_owned()),
            // The row is ahead of the file (a move in flight); index the
            // title now and the body on the next change.
            Err(StoreError::NotFound(_)) => Ok(String::new()),
            Err(error) => Err(SearchError::Source(format!(
                "could not read {path}: {error}"
            ))),
        }
    }
}

#[async_trait]
impl DocumentSource for DriveSource {
    async fn load(&self, doc: DocId) -> Result<Option<IndexDocument>> {
        let node = uuid(doc.as_u128());
        let row: Option<NodeRow> = fetch!(
            self.index,
            "search.feed.node",
            fetch_optional,
            sqlx::query_as(NODE).bind(node)
        );
        let Some((space, kind, path, title, modified, created_by, trash)) = row else {
            return Ok(None);
        };
        if kind != "page" {
            return Ok(None);
        }
        let space = SpaceId::from_u128(space.as_u128());
        let above = self.ancestors(node).await?;
        let in_book = above.iter().any(|(_, kind)| kind == "book");
        let ancestors = std::iter::once(ObjectId::from(space))
            .chain(
                above
                    .iter()
                    .map(|(id, _)| ObjectId::from_u128(id.as_u128())),
            )
            .collect();
        let text = self.text(&path).await?;
        Ok(Some(IndexDocument {
            space,
            doc,
            kind: if in_book {
                DocKind::Book
            } else {
                DocKind::Page
            },
            title,
            owner: created_by.map(|u| UserId::from_u128(u.as_u128())),
            modified: Timestamp::from_unix_ms(modified)
                .map_err(|e| SearchError::Source(e.to_string()))?,
            ancestors,
            trashed: trash.is_some(),
            blocks: crate::markdown::blocks(doc, &text),
            path,
        }))
    }

    async fn list(&self) -> Result<Vec<DocId>> {
        let rows: Vec<(Uuid,)> = fetch!(
            self.index,
            "search.feed.list",
            fetch_all,
            sqlx::query_as(LIST)
        );
        Ok(rows
            .into_iter()
            .map(|(id,)| DocId::from_u128(id.as_u128()))
            .collect())
    }
}

/// What the first sweep does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Queue every page, catching up on edits made while the server was
    /// down. The default.
    CatchUp,
    /// Only remember the pages as they are; use right after a rebuild.
    Current,
}

/// Feed settings.
#[derive(Debug, Clone, Copy)]
pub struct FeedConfig {
    /// Time between sweeps.
    pub interval: Duration,
    /// What the first sweep does.
    pub start: Start,
}

impl Default for FeedConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            start: Start::CatchUp,
        }
    }
}

type SweepRow = (
    Uuid,
    Uuid,
    Option<Uuid>,
    String,
    String,
    i64,
    i64,
    Option<String>,
);

/// Each page's fingerprint as of the last sweep.
#[derive(Debug, Default)]
pub struct Fingerprints(HashMap<DocId, u64>);

impl Fingerprints {
    /// Reads every page's fingerprint from the drive's tables.
    ///
    /// # Errors
    /// When the index cannot be read.
    pub async fn read(index: &Index) -> Result<HashMap<DocId, u64>> {
        let rows: Vec<SweepRow> =
            fetch!(index, "search.feed.sweep", fetch_all, sqlx::query_as(SWEEP));
        Ok(rows
            .into_iter()
            .map(|row| {
                let mut hasher = DefaultHasher::new();
                row.hash(&mut hasher);
                (DocId::from_u128(row.0.as_u128()), hasher.finish())
            })
            .collect())
    }

    /// Takes the new fingerprints and returns the pages that changed,
    /// appeared or disappeared since the last call.
    pub fn update(&mut self, now: HashMap<DocId, u64>) -> Vec<DocId> {
        let mut changed: Vec<DocId> = now
            .iter()
            .filter(|(doc, print)| self.0.get(doc) != Some(print))
            .map(|(doc, _)| *doc)
            .collect();
        changed.extend(self.0.keys().filter(|doc| !now.contains_key(doc)));
        changed.sort_unstable();
        self.0 = now;
        changed
    }
}

/// Watches the drive's pages and feeds changes to an [`Indexer`].
#[derive(Debug)]
pub struct DriveFeed;

impl DriveFeed {
    /// Starts sweeping on the current runtime. The task ends when the
    /// indexer stops; abort the handle to stop it sooner.
    #[must_use]
    pub fn spawn(index: Arc<Index>, indexer: Indexer, config: FeedConfig) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut known = Fingerprints::default();
            let mut first = true;
            let mut tick = tokio::time::interval(config.interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let _ = tick.tick().await;
                let now = match Fingerprints::read(&index).await {
                    Ok(now) => now,
                    Err(error) => {
                        tracing::warn!(%error, "search feed could not read the drive");
                        continue;
                    }
                };
                let changed = known.update(now);
                if std::mem::take(&mut first) && config.start == Start::Current {
                    continue;
                }
                for doc in changed {
                    if indexer.touch(doc).await.is_err() {
                        return;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use vellum_db::IndexConfig;
    use vellum_store::LocalStore;

    use super::*;

    const SPACE: u128 = 0x10;
    const FOLDER: u128 = 0x20;
    const BOOK: u128 = 0x21;
    const PAGE: u128 = 0x30;
    const CHAPTER: u128 = 0x31;
    const ADA: u128 = 0x40;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: std::path::PathBuf,
        index: Arc<Index>,
        source: DriveSource,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("files");
        let index = Arc::new(
            Index::open(&IndexConfig::sqlite(dir.path().join("index.db"), None))
                .await
                .unwrap(),
        );
        let store: Arc<dyn BlobStore> = Arc::new(LocalStore::open(&root).await.unwrap());
        let source = DriveSource::new(Arc::clone(&index), store);
        Fixture {
            _dir: dir,
            root,
            index,
            source,
        }
    }

    fn pool(index: &Index) -> &sqlx::SqlitePool {
        let IndexPool::Sqlite(pool) = index.pool() else {
            panic!("expected SQLite");
        };
        pool
    }

    async fn node(index: &Index, id: u128, kind: &str, parent: Option<u128>, path: &str) {
        let _ = sqlx::query(
            "INSERT INTO drive_nodes (space_id, node_id, kind, parent_id, tree_parent_id, \
             path, title, byte_size, modified_at, created_by, trash) \
             VALUES ($1, $2, $3, $4, $4, $5, $6, 0, 1000, $7, NULL)",
        )
        .bind(uuid(SPACE))
        .bind(uuid(id))
        .bind(kind)
        .bind(parent.map(uuid))
        .bind(path)
        .bind(format!("Title {id:x}"))
        .bind(uuid(ADA))
        .execute(pool(index))
        .await
        .unwrap();
    }

    async fn sql(index: &Index, statement: &'static str) {
        let _ = sqlx::query(statement).execute(pool(index)).await.unwrap();
    }

    fn write(root: &std::path::Path, path: &str, text: &str) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[tokio::test]
    async fn loads_a_saved_page_with_its_place_and_blocks() {
        let f = fixture().await;
        node(&f.index, FOLDER, "folder", None, "eng/plans").await;
        node(&f.index, PAGE, "page", Some(FOLDER), "eng/plans/launch.md").await;
        write(
            &f.root,
            "eng/plans/launch.md",
            "# Launch\n\nShip on Friday.\n",
        );

        let doc = f
            .source
            .load(DocId::from_u128(PAGE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.space, SpaceId::from_u128(SPACE));
        assert_eq!(doc.kind, DocKind::Page);
        assert_eq!(doc.title, "Title 30");
        assert_eq!(doc.path, "eng/plans/launch.md");
        assert_eq!(doc.owner, Some(UserId::from_u128(ADA)));
        assert_eq!(doc.modified.unix_ms(), 1000);
        assert_eq!(
            doc.ancestors,
            vec![
                ObjectId::from(SpaceId::from_u128(SPACE)),
                ObjectId::from_u128(FOLDER)
            ]
        );
        assert!(!doc.trashed);
        let texts: Vec<_> = doc.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, vec!["Launch", "Ship on Friday."]);

        assert_eq!(f.source.list().await.unwrap(), vec![DocId::from_u128(PAGE)]);
    }

    #[tokio::test]
    async fn folders_missing_files_trash_and_books() {
        let f = fixture().await;
        node(&f.index, FOLDER, "folder", None, "eng").await;
        node(&f.index, BOOK, "book", Some(FOLDER), "eng/guide").await;
        node(&f.index, CHAPTER, "page", Some(BOOK), "eng/guide/one.md").await;

        assert_eq!(f.source.load(DocId::from_u128(FOLDER)).await.unwrap(), None);
        assert_eq!(f.source.load(DocId::from_u128(0x99)).await.unwrap(), None);

        // No file yet: the title is still searchable.
        let chapter = f
            .source
            .load(DocId::from_u128(CHAPTER))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chapter.kind, DocKind::Book);
        assert!(chapter.blocks.is_empty());
        assert_eq!(chapter.ancestors.len(), 3);

        sql(&f.index, "UPDATE drive_nodes SET trash = '{}'").await;
        let trashed = f
            .source
            .load(DocId::from_u128(CHAPTER))
            .await
            .unwrap()
            .unwrap();
        assert!(trashed.trashed);
    }

    #[tokio::test]
    async fn sweeps_report_new_changed_and_removed_pages() {
        let f = fixture().await;
        let mut known = Fingerprints::default();
        assert!(
            known
                .update(Fingerprints::read(&f.index).await.unwrap())
                .is_empty()
        );

        node(&f.index, FOLDER, "folder", None, "eng").await;
        node(&f.index, PAGE, "page", Some(FOLDER), "eng/a.md").await;
        node(&f.index, CHAPTER, "page", Some(FOLDER), "eng/b.md").await;
        let page = DocId::from_u128(PAGE);
        let chapter = DocId::from_u128(CHAPTER);
        assert_eq!(
            known.update(Fingerprints::read(&f.index).await.unwrap()),
            vec![page, chapter]
        );
        assert!(
            known
                .update(Fingerprints::read(&f.index).await.unwrap())
                .is_empty()
        );

        sql(
            &f.index,
            "UPDATE drive_nodes SET modified_at = 2000 WHERE path = 'eng/a.md'",
        )
        .await;
        assert_eq!(
            known.update(Fingerprints::read(&f.index).await.unwrap()),
            vec![page]
        );

        // Renaming the folder moves both pages.
        sql(
            &f.index,
            "UPDATE drive_nodes SET path = replace(path, 'eng', 'team')",
        )
        .await;
        assert_eq!(
            known.update(Fingerprints::read(&f.index).await.unwrap()),
            vec![page, chapter]
        );

        // Purging the folder removes both pages.
        sql(&f.index, "DELETE FROM drive_nodes").await;
        assert_eq!(
            known.update(Fingerprints::read(&f.index).await.unwrap()),
            vec![page, chapter]
        );
    }
}
