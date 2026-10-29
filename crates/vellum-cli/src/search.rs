// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Search in `vellum serve`: the index, the indexer and what feeds it.
//!
//! - The index is embedded tantivy in `<data>/.vellum/search`, sealed with
//!   a key derived from `VELLUM_INDEX_KEY` (plaintext in plaintext mode),
//!   or Postgres full-text search when the server runs on Postgres.
//! - Drive pages reach it through `vellum_search::DriveFeed`, which follows
//!   the page change log. With the shared Postgres tables one replica of the
//!   cluster follows it at a time (ADR 0038, D164). Collection rows reach it through the collections
//!   service's index feed, so formulas and rollups are searchable too.
//! - A row has no node of its own in the permission tree, so a row hit is
//!   checked against its collection: whoever can read the collection can
//!   find its rows.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use hkdf::Hkdf;
use sha2::Sha256;
use tokio::sync::mpsc;
use vellum_authz::{Authz, AuthzStore};
use vellum_collections::CollectionsService;
use vellum_collections::service::{IndexFeed, RowDocument, SpaceOf};
use vellum_db::{Index, IndexPool};
use vellum_kernel::{DocId, Error, ErrorKind, ObjectId, SpaceId, Timestamp, UserId};
use vellum_search::{
    DocKind, DocumentSource, DriveFeed, DriveSource, FeedConfig, IndexDocument, IndexKey, Indexer,
    IndexerConfig, Location, Opened, PostgresBackend, Principal, ReadAccess, SearchApi,
    SearchBackend, SearchService, Sharded, Stars, Start, TantivyBackend, TantivyConfig,
};
use vellum_store::{BlobPath, BlobStore};

/// Context string for the search key, so it never equals the index key.
const KEY_INFO: &[u8] = b"vellum search index key v1";

/// Documents written per batch during a rebuild.
const REBUILD_BATCH: usize = 64;

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// The key that seals the search index, derived from the index key.
///
/// The two indexes use different ciphers, so they get different keys.
#[must_use]
pub fn search_key(index_key: &[u8; 32]) -> IndexKey {
    IndexKey::from_bytes(derive(index_key))
}

fn derive(index_key: &[u8; 32]) -> [u8; 32] {
    let mut key = [0_u8; 32];
    // HKDF-SHA256 can produce up to 8160 bytes, so 32 never fails.
    let _ = Hkdf::<Sha256>::new(None, index_key).expand(KEY_INFO, &mut key);
    key
}

/// Which collection each indexed row belongs to.
#[derive(Debug, Default)]
pub struct RowOwners(RwLock<HashMap<DocId, DocId>>);

impl RowOwners {
    fn set(&self, row: DocId, collection: Option<DocId>) {
        if let Ok(mut rows) = self.0.write() {
            match collection {
                Some(c) => drop(rows.insert(row, c)),
                None => drop(rows.remove(&row)),
            }
        }
    }

    /// The node to check for `doc`: its collection for a row, else itself.
    fn node(&self, doc: DocId) -> DocId {
        self.0
            .read()
            .ok()
            .and_then(|rows| rows.get(&doc).copied())
            .unwrap_or(doc)
    }
}

/// Read checks that send a row to its collection's node.
pub struct RowAwareAccess<S: AuthzStore + 'static> {
    authz: Arc<Authz<S>>,
    rows: Arc<RowOwners>,
}

#[async_trait]
impl<S: AuthzStore + 'static> ReadAccess for RowAwareAccess<S> {
    async fn readable_spaces(&self, who: &Principal) -> vellum_search::Result<Vec<SpaceId>> {
        self.authz.readable_spaces(who).await
    }

    async fn filter_readable(
        &self,
        who: &Principal,
        docs: &[DocId],
    ) -> vellum_search::Result<HashSet<DocId>> {
        let nodes: Vec<DocId> = docs.iter().map(|d| self.rows.node(*d)).collect();
        let allowed = self.authz.filter_readable(who, &nodes).await?;
        Ok(docs
            .iter()
            .zip(&nodes)
            .filter(|(_, node)| allowed.contains(node))
            .map(|(doc, _)| *doc)
            .collect())
    }
}

/// Nobody has starred anything; for printing the API contracts.
struct NoStars;

#[async_trait]
impl Stars for NoStars {
    async fn starred(&self, _: UserId) -> vellum_search::Result<HashSet<DocId>> {
        Ok(HashSet::new())
    }
}

/// Drive pages, plus collection rows with their computed values.
struct WorkspaceSource {
    /// Pages, from every shard's drive tables.
    drive: Sharded<DriveSource>,
    collections: Arc<CollectionsService>,
    rows: Arc<RowOwners>,
    space_of: SpaceOf,
}

/// A row as an index document: its properties as `Name: value` lines,
/// then its page body.
fn row_document(row: RowDocument, space: SpaceId) -> IndexDocument {
    let mut text = String::new();
    for field in &row.fields {
        text.push_str(&field.name);
        text.push_str(": ");
        text.push_str(&field.text);
        text.push_str("\n\n");
    }
    text.push_str(&row.body);
    IndexDocument {
        space,
        doc: row.row,
        kind: DocKind::Row,
        title: row.title,
        path: row.path,
        owner: None,
        // Rows carry no modified time of their own: computed values change
        // without a file write, so "now" is when the row was last indexed.
        modified: Timestamp::now(),
        ancestors: vec![
            ObjectId::from(space),
            ObjectId::from_u128(row.collection.as_u128()),
        ],
        trashed: false,
        blocks: vellum_search::markdown::blocks(row.row, &text),
    }
}

fn source_error(e: impl std::fmt::Display) -> vellum_search::SearchError {
    vellum_search::SearchError::Source(e.to_string())
}

#[async_trait]
impl DocumentSource for WorkspaceSource {
    async fn load(&self, doc: DocId) -> vellum_search::Result<Option<IndexDocument>> {
        if let Some(row) = self
            .collections
            .index_row(doc)
            .await
            .map_err(source_error)?
        {
            let space = BlobPath::new(&row.path)
                .ok()
                .and_then(|path| (self.space_of)(&path));
            if let Some(space) = space {
                self.rows
                    .set(doc, Some(DocId::from_u128(row.collection.as_u128())));
                return Ok(Some(row_document(row, SpaceId::from_u128(space.0))));
            }
            // Outside every space: nobody could read it, so leave it out.
            self.rows.set(doc, None);
            return Ok(None);
        }
        self.rows.set(doc, None);
        self.drive.load(doc).await
    }

    async fn list(&self) -> vellum_search::Result<Vec<DocId>> {
        let mut docs = self.drive.list().await?;
        docs.extend(self.collections.index_rows().await.map_err(source_error)?);
        Ok(docs)
    }
}

/// Hands the rows a collections write touched to the indexer.
///
/// The collections service calls this under its write lock, so it only
/// queues them; [`start`] forwards them to the indexer.
pub struct RowFeed(mpsc::UnboundedSender<Vec<DocId>>);

impl IndexFeed for RowFeed {
    fn rows_changed(&self, rows: Vec<DocId>) {
        // A closed channel means search is off; there is nothing to do.
        let _ = self.0.send(rows);
    }
}

/// A row feed, and the receiving end [`start`] drains.
#[must_use]
pub fn row_feed() -> (Arc<RowFeed>, mpsc::UnboundedReceiver<Vec<DocId>>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Arc::new(RowFeed(tx)), rx)
}

/// Everything [`start`] needs.
pub struct Parts<S: AuthzStore + 'static> {
    /// The data folder.
    pub data_dir: std::path::PathBuf,
    /// The raw index key; `None` in plaintext mode.
    pub key: Option<[u8; 32]>,
    /// The index database: the search tables on Postgres.
    pub index: Index,
    /// Every shard's database, holding the drive's tables (just `index`
    /// without shards).
    pub shards: Vec<Index>,
    /// The files.
    pub store: Arc<dyn BlobStore>,
    /// Permissions.
    pub authz: Arc<Authz<S>>,
    /// Collections, for rows.
    pub collections: Arc<CollectionsService>,
    /// Path to space lookup.
    pub space_of: SpaceOf,
    /// The rows collections writes touch.
    pub row_changes: mpsc::UnboundedReceiver<Vec<DocId>>,
    /// This replica's name, for the shared feed's lease.
    pub holder: String,
}

async fn backend(
    data_dir: &Path,
    key: Option<[u8; 32]>,
    index: &Index,
) -> Result<(Arc<dyn SearchBackend>, Opened), Error> {
    if let IndexPool::Postgres(pool) = index.pool() {
        let backend = PostgresBackend::connect(pool.clone())
            .await
            .map_err(|e| unavailable("cannot open the search tables", e))?;
        // The tables are durable; the startup sweep catches up on changes.
        return Ok((Arc::new(backend), Opened::Ready));
    }
    let config = TantivyConfig {
        location: Location::Folder(data_dir.join(".vellum").join("search")),
        key: key.as_ref().map(search_key),
        model: None,
    };
    let (backend, opened) = tokio::task::spawn_blocking(move || TantivyBackend::open(&config))
        .await
        .map_err(|e| unavailable("search index task failed", e))?
        .map_err(|e| unavailable("cannot open the search index", e))?;
    Ok((Arc::new(backend), opened))
}

/// Opens the search index and starts keeping it up to date.
///
/// A missing or stale index is rebuilt in the background, so the server
/// starts at once; until the rebuild ends, results may be incomplete.
///
/// # Errors
///
/// When the index cannot be opened.
pub async fn start<S: AuthzStore + 'static>(parts: Parts<S>) -> Result<SearchApi, Error> {
    let Parts {
        data_dir,
        key,
        index,
        shards,
        store,
        authz,
        collections,
        space_of,
        mut row_changes,
        holder,
    } = parts;
    let shared = matches!(index.pool(), IndexPool::Postgres(_));
    let (backend, opened) = backend(&data_dir, key, &index).await?;
    let rows = Arc::new(RowOwners::default());
    let service = SearchService::new(
        backend,
        Arc::new(RowAwareAccess {
            authz,
            rows: Arc::clone(&rows),
        }),
        Arc::new(Sharded(shards.clone())) as Arc<dyn Stars>,
    );
    let source = Arc::new(WorkspaceSource {
        drive: Sharded(
            shards
                .iter()
                .map(|shard| DriveSource::new(Arc::new(shard.clone()), Arc::clone(&store)))
                .collect(),
        ),
        collections: Arc::clone(&collections),
        rows,
        space_of,
    });
    let (indexer, _task) = Indexer::spawn(
        service.clone(),
        Arc::clone(&source) as Arc<dyn DocumentSource>,
        IndexerConfig::default(),
    );
    let forward = indexer.clone();
    drop(tokio::spawn(async move {
        while let Some(batch) = row_changes.recv().await {
            for row in batch {
                if forward.touch(row).await.is_err() {
                    return;
                }
            }
        }
    }));
    let rebuild_service = service.clone();
    drop(tokio::spawn(async move {
        let start = if matches!(opened, Opened::NeedsRebuild) {
            match vellum_search::rebuild(&rebuild_service, source.as_ref(), REBUILD_BATCH).await {
                Ok(done) => {
                    tracing::info!(documents = done.documents, "search index rebuilt");
                    Start::Current
                }
                Err(error) => {
                    tracing::warn!(%error, "search rebuild failed; indexing page by page");
                    Start::CatchUp
                }
            }
        } else {
            // Rows can change while the server is down (a formula's inputs
            // edited on disk), so they are always indexed again.
            match collections.index_rows().await {
                Ok(all) => {
                    for row in all {
                        if indexer.touch(row).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "collection rows not reindexed"),
            }
            Start::CatchUp
        };
        let config = FeedConfig {
            start,
            ..FeedConfig::default()
        };
        // Each shard logs its own page changes. A shared index needs one
        // follower per shard in the cluster; a local one needs one per
        // replica.
        for shard in shards {
            let (db, indexer) = (Arc::new(shard), indexer.clone());
            drop(if shared {
                DriveFeed::spawn_shared(db, indexer, config, &holder)
            } else {
                DriveFeed::spawn(db, indexer, config)
            });
        }
    }));
    Ok(SearchApi::new(service))
}

/// The search API over an empty in-memory index, for printing the API
/// contracts.
///
/// # Errors
///
/// When the in-memory index cannot be created.
pub fn describe<S: AuthzStore + 'static>(authz: Arc<Authz<S>>) -> Result<SearchApi, Error> {
    let (backend, _) = TantivyBackend::open(&TantivyConfig {
        location: Location::Memory,
        key: None,
        model: None,
    })
    .map_err(|e| unavailable("cannot open an in-memory search index", e))?;
    let service = SearchService::new(
        Arc::new(backend),
        Arc::new(RowAwareAccess {
            authz,
            rows: Arc::default(),
        }),
        Arc::new(NoStars),
    );
    Ok(SearchApi::new(service))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vellum_authz::{Actor, Change, Id, MemoryStore, Object, Relation, Subject, Tuple};
    use vellum_collections::service::{Access, Need};
    use vellum_db::IndexConfig;
    use vellum_store::LocalStore;

    use super::*;

    const COLLECTION: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C30";
    const ROW: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C41";

    /// The indexer needs no access checks from collections.
    struct Nobody;

    #[async_trait]
    impl Access for Nobody {
        async fn allowed(
            &self,
            _: &vellum_contracts::Principal,
            _: vellum_kernel::CollectionId,
            _: &BlobPath,
            _: Need,
        ) -> Result<bool, Error> {
            Ok(false)
        }
    }

    #[test]
    fn the_search_key_differs_from_the_index_key() {
        let raw = [7_u8; 32];
        assert_ne!(derive(&raw), raw);
        assert_ne!(derive(&raw), [0_u8; 32]);
        assert_eq!(derive(&raw), derive(&raw));
        assert_ne!(derive(&raw), derive(&[8_u8; 32]));
    }

    #[test]
    fn rows_are_checked_against_their_collection() {
        let rows = RowOwners::default();
        let row = DocId::from_u128(1);
        let collection = DocId::from_u128(2);
        assert_eq!(rows.node(row), row);
        rows.set(row, Some(collection));
        assert_eq!(rows.node(row), collection);
        rows.set(row, None);
        assert_eq!(rows.node(row), row);
    }

    #[tokio::test]
    async fn rows_are_indexed_with_their_values_and_readable_through_the_collection() {
        let dir = std::env::temp_dir().join(format!("vellum-search-rows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("drives/shared/Northstar/Vendors.collection");
        std::fs::create_dir_all(folder.join("rows")).unwrap();
        std::fs::write(
            folder.join("schema.yaml"),
            format!(
                "id: {COLLECTION}\ntitle: Vendors\nproperties:\n  - {{key: t, name: Name, type: title}}\n  - {{key: city, name: City, type: text}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            folder.join("rows/acme.md"),
            format!("---\nid: {ROW}\nt: Acme\ncity: Zanzibar\n---\nShips on Fridays.\n"),
        )
        .unwrap();
        let store: Arc<dyn BlobStore> = Arc::new(LocalStore::open(&dir).await.unwrap());
        let index = Index::open(&IndexConfig::sqlite(dir.join("index.sqlite"), None))
            .await
            .unwrap();
        let collections = Arc::new(CollectionsService::new(
            Arc::clone(&store),
            Arc::new(Nobody),
        ));
        let rows = Arc::new(RowOwners::default());
        let space = Id(5);
        let source = WorkspaceSource {
            drive: Sharded(vec![DriveSource::new(Arc::new(index), store)]),
            collections,
            rows: Arc::clone(&rows),
            space_of: Arc::new(move |_: &BlobPath| Some(space)),
        };
        let row: DocId = ROW.parse().unwrap();
        let collection: DocId = COLLECTION.parse().unwrap();

        assert!(source.list().await.unwrap().contains(&row));
        let doc = source.load(row).await.unwrap().unwrap();
        assert_eq!(doc.kind, DocKind::Row);
        assert_eq!(doc.title, "Acme");
        assert_eq!(doc.space, SpaceId::from_u128(5));
        let text: Vec<&str> = doc.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(text.contains(&"City: Zanzibar"), "{text:?}");
        assert!(text.contains(&"Ships on Fridays."), "{text:?}");

        // A space viewer can read the collection, so finds the row; the
        // row itself has no node in the permission tree.
        let store = MemoryStore::new();
        store
            .set_tree(space, BTreeMap::from([(Id::from(collection), None)]))
            .unwrap();
        let authz = Arc::new(Authz::new(store));
        let ada = UserId::from_u128(9);
        let _ = authz
            .write(
                space,
                Actor::System,
                &[Change::Grant(Tuple::role(
                    Object::Space(space),
                    Relation::Viewer,
                    Subject::User(ada.into()),
                ))],
            )
            .await
            .unwrap();
        let who = Principal::User(ada);
        let access = RowAwareAccess {
            authz: Arc::clone(&authz),
            rows,
        };
        assert!(
            access
                .filter_readable(&who, &[row])
                .await
                .unwrap()
                .contains(&row)
        );
        let unaware = RowAwareAccess {
            authz,
            rows: Arc::default(),
        };
        assert!(
            unaware
                .filter_readable(&who, &[row])
                .await
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
