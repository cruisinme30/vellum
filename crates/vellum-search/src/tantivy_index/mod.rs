// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The embedded tantivy backend for the single binary (D23).
//!
//! One tantivy document per block (`rec = b`) plus one title record per
//! document (`rec = d`) for the instant title search. Vectors live in a
//! bytes fast field next to the block, so a filtered nearest-neighbour scan
//! and BM25 see the same commit. The writer is opened per batch and
//! dropped after it, so an idle server holds no indexing arena (R-LT-2).

mod crypt;
mod schema;

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tantivy::collector::{Collector, SegmentCollector, TopDocs};
use tantivy::columnar::BytesColumn;
use tantivy::directory::{MmapDirectory, RamDirectory};
use tantivy::query::{
    BooleanQuery, ConstScoreQuery, Occur, Query, QueryParser, RangeQuery, RegexQuery, TermQuery,
    TermSetQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy::{
    DocAddress, Index, IndexReader, ReloadPolicy, Searcher, SegmentReader, TantivyDocument, Term,
};
use vellum_kernel::{BlockId, DocId};

pub use self::crypt::{EncryptedDirectory, IndexKey};
pub use self::schema::plain_tokens as plain_words;
use self::schema::{Fields, NO_SECTION, REC_BLOCK, REC_DOC};
use crate::backend::{Change, Scope, SearchBackend};
use crate::error::{Result, SearchError};
use crate::model::{StoredBlock, TitleHit};
use crate::vector;

/// The indexing arena per batch; tantivy's minimum is 15 MiB.
const WRITER_ARENA: usize = 16 << 20;

/// The most blocks one section can return.
const MAX_SECTION: usize = 5_000;

/// The file that records the layout version and embedding model.
const STAMP: &str = "vellum-search.json";

/// Bumped whenever the schema or analysis changes; a mismatch rebuilds.
const LAYOUT_VERSION: u32 = 1;

/// Where the tantivy index lives.
#[derive(Debug, Clone)]
pub enum Location {
    /// In memory (tests and ephemeral servers).
    Memory,
    /// In a folder on disk.
    Folder(PathBuf),
}

/// How to open the index.
#[derive(Debug, Clone)]
pub struct TantivyConfig {
    /// Where the index lives.
    pub location: Location,
    /// The encryption key, or `None` for plaintext mode (only when every
    /// space is plaintext, D63).
    pub key: Option<IndexKey>,
    /// The embedding model in use, if any. A change empties the index.
    pub model: Option<String>,
}

/// The embedded tantivy index.
#[derive(Clone)]
pub struct TantivyBackend {
    inner: Arc<Inner>,
}

struct Inner {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    writer_lock: Mutex<()>,
}

/// What opening the index found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The index was usable as is.
    Ready,
    /// The index was missing or stale and is now empty: rebuild it.
    NeedsRebuild,
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Stamp {
    layout: u32,
    model: Option<String>,
}

impl TantivyBackend {
    /// Opens (or creates) the index. A layout or model change empties it,
    /// and the result says a rebuild from the files is needed.
    ///
    /// # Errors
    /// File-system, encryption or tantivy errors.
    pub fn open(config: &TantivyConfig) -> Result<(Self, Opened)> {
        let want = Stamp {
            layout: LAYOUT_VERSION,
            model: config.model.clone(),
        };
        let mut opened = Opened::Ready;
        let (index, fields) = match &config.location {
            Location::Memory => {
                opened = Opened::NeedsRebuild;
                let directory = wrap(Box::new(RamDirectory::create()), config.key.as_ref());
                create(directory, &want)?
            }
            Location::Folder(path) => {
                std::fs::create_dir_all(path)?;
                let directory = wrap(
                    Box::new(MmapDirectory::open(path).map_err(open_dir)?),
                    config.key.as_ref(),
                );
                let stamp = directory
                    .atomic_read(std::path::Path::new(STAMP))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Stamp>(&bytes).ok());
                if stamp.as_ref() == Some(&want) {
                    let index = Index::open(directory)?;
                    let fields = Fields::resolve(&index.schema())?;
                    schema::register(&index);
                    (index, fields)
                } else {
                    // Missing, unreadable (wrong key) or stale: start empty.
                    opened = Opened::NeedsRebuild;
                    drop(directory);
                    std::fs::remove_dir_all(path)?;
                    std::fs::create_dir_all(path)?;
                    let directory = wrap(
                        Box::new(MmapDirectory::open(path).map_err(open_dir)?),
                        config.key.as_ref(),
                    );
                    create(directory, &want)?
                }
            }
        };
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let backend = Self {
            inner: Arc::new(Inner {
                index,
                reader,
                fields,
                writer_lock: Mutex::new(()),
            }),
        };
        Ok((backend, opened))
    }

    async fn blocking<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> Result<T> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || work(&inner))
            .await
            .map_err(|error| SearchError::Corrupt(format!("search task failed: {error}")))?
    }
}

fn open_dir(error: tantivy::directory::error::OpenDirectoryError) -> SearchError {
    SearchError::Io(std::io::Error::other(error.to_string()))
}

fn wrap(
    directory: Box<dyn tantivy::directory::Directory>,
    key: Option<&IndexKey>,
) -> Box<dyn tantivy::directory::Directory> {
    match key {
        Some(key) => Box::new(EncryptedDirectory::new(directory, key)),
        None => directory,
    }
}

fn create(
    directory: Box<dyn tantivy::directory::Directory>,
    stamp: &Stamp,
) -> Result<(Index, Fields)> {
    let (schema, fields) = schema::build();
    let bytes = serde_json::to_vec(stamp).map_err(|e| SearchError::Corrupt(e.to_string()))?;
    directory.atomic_write(std::path::Path::new(STAMP), &bytes)?;
    let index = Index::create(directory, schema, tantivy::IndexSettings::default())?;
    schema::register(&index);
    Ok((index, fields))
}

impl Inner {
    fn searcher(&self) -> Searcher {
        self.reader.searcher()
    }

    fn term(&self, field: tantivy::schema::Field, text: &str) -> Box<dyn Query> {
        Box::new(TermQuery::new(
            Term::from_field_text(field, text),
            IndexRecordOption::Basic,
        ))
    }

    /// The scope as filter clauses that add no score.
    fn filter(&self, rec: &str, scope: Scope<'_>) -> Vec<(Occur, Box<dyn Query>)> {
        let f = &self.fields;
        let mut clauses: Vec<Box<dyn Query>> = vec![
            self.term(f.rec, rec),
            Box::new(TermSetQuery::new(
                scope
                    .spaces
                    .iter()
                    .map(|space| Term::from_field_text(f.space, &space.to_string())),
            )),
            Box::new(TermQuery::new(
                Term::from_field_u64(f.trashed, u64::from(scope.filters.trashed)),
                IndexRecordOption::Basic,
            )),
        ];
        if let Some(docs) = scope.docs {
            clauses.push(Box::new(TermSetQuery::new(
                docs.iter()
                    .map(|doc| Term::from_field_text(f.doc, &doc.to_string())),
            )));
        }
        if !scope.filters.owners.is_empty() {
            clauses.push(Box::new(TermSetQuery::new(
                scope
                    .filters
                    .owners
                    .iter()
                    .map(|owner| Term::from_field_text(f.owner, &owner.to_string())),
            )));
        }
        if !scope.filters.kinds.is_empty() {
            clauses.push(Box::new(TermSetQuery::new(
                scope
                    .filters
                    .kinds
                    .iter()
                    .map(|kind| Term::from_field_text(f.doc_kind, kind.as_str())),
            )));
        }
        if let Some(range) = &scope.filters.modified {
            clauses.push(Box::new(RangeQuery::new(
                std::ops::Bound::Included(Term::from_field_i64(f.modified, *range.start())),
                std::ops::Bound::Included(Term::from_field_i64(f.modified, *range.end())),
            )));
        }
        if let Some(location) = scope.filters.location {
            clauses.push(self.term(f.ancestor, &location.to_string()));
        }
        clauses
            .into_iter()
            .map(|query| {
                (
                    Occur::Must,
                    Box::new(ConstScoreQuery::new(query, 0.0)) as Box<dyn Query>,
                )
            })
            .collect()
    }

    fn stored_block(&self, searcher: &Searcher, address: DocAddress) -> Result<StoredBlock> {
        let document: TantivyDocument = searcher.doc(address)?;
        schema::read_block(&self.fields, &document)
    }

    fn apply(&self, changes: &[Change]) -> Result<()> {
        let _guard = self
            .writer_lock
            .lock()
            .map_err(|_| SearchError::Corrupt("writer lock poisoned".into()))?;
        let mut writer = self
            .index
            .writer_with_num_threads::<TantivyDocument>(1, WRITER_ARENA)?;
        for change in changes {
            match change {
                Change::Remove(doc) => {
                    let _ = writer
                        .delete_term(Term::from_field_text(self.fields.doc, &doc.to_string()));
                }
                Change::Upsert(entry) => {
                    let _ = writer.delete_term(Term::from_field_text(
                        self.fields.doc,
                        &entry.doc.doc.to_string(),
                    ));
                    for document in schema::documents(&self.fields, entry)? {
                        let _ = writer.add_document(document)?;
                    }
                }
            }
        }
        let _ = writer.commit()?;
        writer.wait_merging_threads()?;
        self.reader.reload()?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let _guard = self
            .writer_lock
            .lock()
            .map_err(|_| SearchError::Corrupt("writer lock poisoned".into()))?;
        let mut writer = self
            .index
            .writer_with_num_threads::<TantivyDocument>(1, WRITER_ARENA)?;
        let _ = writer.delete_all_documents()?;
        let _ = writer.commit()?;
        writer.wait_merging_threads()?;
        self.reader.reload()?;
        Ok(())
    }

    fn lexical(&self, text: &str, scope: Scope<'_>, limit: usize) -> Result<Vec<StoredBlock>> {
        let f = &self.fields;
        let mut parser = QueryParser::for_index(&self.index, vec![f.title, f.headings, f.body]);
        parser.set_field_boost(f.title, 2.0);
        parser.set_field_boost(f.headings, 1.5);
        // Lenient: whatever a person types is a query, never an error.
        let (user_query, _errors) = parser.parse_query_lenient(text);
        let mut clauses = self.filter(REC_BLOCK, scope);
        clauses.push((Occur::Must, user_query));
        let query = BooleanQuery::new(clauses);
        let searcher = self.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit.max(1)).order_by_score())?;
        top.into_iter()
            .map(|(_, address)| self.stored_block(&searcher, address))
            .collect()
    }

    fn semantic(
        &self,
        vector: &[f32],
        min_similarity: f32,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        let query = BooleanQuery::new(self.filter(REC_BLOCK, scope));
        let searcher = self.searcher();
        let collector = Nearest {
            query: vector.to_vec(),
            min_similarity,
            limit: limit.max(1),
        };
        let nearest = searcher.search(&query, &collector)?;
        nearest
            .into_iter()
            .map(|scored| self.stored_block(&searcher, scored.address))
            .collect()
    }

    fn titles(&self, text: &str, scope: Scope<'_>, limit: usize) -> Result<Vec<TitleHit>> {
        let f = &self.fields;
        let words = schema::plain_tokens(text);
        let Some((last, complete)) = words.split_last() else {
            return Ok(Vec::new());
        };
        let mut clauses = self.filter(REC_DOC, scope);
        for word in complete {
            clauses.push((Occur::Must, self.term(f.title_plain, word)));
        }
        let prefix = format!("{}.*", regex_escape(last));
        clauses.push((
            Occur::Must,
            Box::new(RegexQuery::from_pattern(&prefix, f.title_plain)?),
        ));
        let query = BooleanQuery::new(clauses);
        let searcher = self.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit.max(1)).order_by_score())?;
        top.into_iter()
            .map(|(_, address)| {
                let document: TantivyDocument = searcher.doc(address)?;
                schema::read_title(f, &document)
            })
            .collect()
    }

    fn block(&self, doc: DocId, block: BlockId) -> Result<Option<StoredBlock>> {
        let f = &self.fields;
        let query = BooleanQuery::new(vec![
            (Occur::Must, self.term(f.rec, REC_BLOCK)),
            (Occur::Must, self.term(f.doc, &doc.to_string())),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(f.block, block.as_u64()),
                    IndexRecordOption::Basic,
                )),
            ),
        ]);
        let searcher = self.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(1).order_by_score())?;
        top.first()
            .map(|(_, address)| self.stored_block(&searcher, *address))
            .transpose()
    }

    fn section(&self, doc: DocId, section: Option<BlockId>) -> Result<Vec<StoredBlock>> {
        let f = &self.fields;
        let key = section.map_or(NO_SECTION, BlockId::as_u64);
        let query = BooleanQuery::new(vec![
            (Occur::Must, self.term(f.rec, REC_BLOCK)),
            (Occur::Must, self.term(f.doc, &doc.to_string())),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(f.section, key),
                    IndexRecordOption::Basic,
                )),
            ),
        ]);
        let searcher = self.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(MAX_SECTION).order_by_score())?;
        let mut blocks = top
            .into_iter()
            .map(|(_, address)| self.stored_block(&searcher, address))
            .collect::<Result<Vec<_>>>()?;
        blocks.sort_by_key(|block| block.position);
        Ok(blocks)
    }
}

/// Escapes regex metacharacters (tokens are alphanumeric, so rarely needed).
fn regex_escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if !c.is_alphanumeric() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[async_trait]
impl SearchBackend for TantivyBackend {
    fn name(&self) -> &'static str {
        "tantivy"
    }

    async fn apply(&self, changes: &[Change]) -> Result<()> {
        let changes = changes.to_vec();
        self.blocking(move |inner| inner.apply(&changes)).await
    }

    async fn clear(&self) -> Result<()> {
        self.blocking(Inner::clear).await
    }

    async fn lexical(
        &self,
        text: &str,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        let owned = OwnedScope::from(scope);
        let text = text.to_owned();
        self.blocking(move |inner| inner.lexical(&text, owned.scope(), limit))
            .await
    }

    async fn semantic(
        &self,
        vector: &[f32],
        min_similarity: f32,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        let owned = OwnedScope::from(scope);
        let vector = vector.to_vec();
        self.blocking(move |inner| inner.semantic(&vector, min_similarity, owned.scope(), limit))
            .await
    }

    async fn titles(&self, text: &str, scope: Scope<'_>, limit: usize) -> Result<Vec<TitleHit>> {
        let owned = OwnedScope::from(scope);
        let text = text.to_owned();
        self.blocking(move |inner| inner.titles(&text, owned.scope(), limit))
            .await
    }

    async fn block(&self, doc: DocId, block: BlockId) -> Result<Option<StoredBlock>> {
        self.blocking(move |inner| inner.block(doc, block)).await
    }

    async fn section(&self, doc: DocId, section: Option<BlockId>) -> Result<Vec<StoredBlock>> {
        self.blocking(move |inner| inner.section(doc, section))
            .await
    }
}

/// A [`Scope`] copied so it can move to a blocking thread.
struct OwnedScope {
    spaces: Vec<vellum_kernel::SpaceId>,
    docs: Option<std::collections::HashSet<DocId>>,
    filters: crate::model::Filters,
}

impl From<Scope<'_>> for OwnedScope {
    fn from(scope: Scope<'_>) -> Self {
        Self {
            spaces: scope.spaces.to_vec(),
            docs: scope.docs.cloned(),
            filters: scope.filters.clone(),
        }
    }
}

impl OwnedScope {
    fn scope(&self) -> Scope<'_> {
        Scope {
            spaces: &self.spaces,
            docs: self.docs.as_ref(),
            filters: &self.filters,
        }
    }
}

/// A scored hit from the nearest-neighbour scan.
#[derive(Debug, Clone, Copy)]
struct ScoredAddress {
    score: f32,
    address: DocAddress,
}

impl PartialEq for ScoredAddress {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ScoredAddress {}

impl PartialOrd for ScoredAddress {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredAddress {
    /// Reversed, so a max-heap keeps the worst hit on top for eviction.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.address.cmp(&other.address))
    }
}

/// Exact nearest neighbours over the filtered blocks of each segment.
struct Nearest {
    query: Vec<f32>,
    min_similarity: f32,
    limit: usize,
}

struct SegmentNearest {
    query: Vec<f32>,
    min_similarity: f32,
    limit: usize,
    segment: u32,
    column: Option<BytesColumn>,
    heap: BinaryHeap<ScoredAddress>,
    buffer: Vec<u8>,
    error: Option<String>,
}

impl Collector for Nearest {
    type Fruit = Vec<ScoredAddress>;
    type Child = SegmentNearest;

    fn for_segment(&self, segment: u32, reader: &SegmentReader) -> tantivy::Result<SegmentNearest> {
        Ok(SegmentNearest {
            query: self.query.clone(),
            min_similarity: self.min_similarity,
            limit: self.limit,
            segment,
            column: reader.fast_fields().bytes(schema::VEC)?,
            heap: BinaryHeap::with_capacity(self.limit + 1),
            buffer: Vec::new(),
            error: None,
        })
    }

    fn requires_scoring(&self) -> bool {
        false
    }

    fn merge_fruits(&self, fruits: Vec<Vec<ScoredAddress>>) -> tantivy::Result<Vec<ScoredAddress>> {
        let mut all: Vec<ScoredAddress> = fruits.into_iter().flatten().collect();
        // `Ord` is reversed, so ascending order is best first.
        all.sort();
        all.truncate(self.limit);
        Ok(all)
    }
}

impl SegmentCollector for SegmentNearest {
    type Fruit = Vec<ScoredAddress>;

    fn collect(&mut self, doc: tantivy::DocId, _score: tantivy::Score) {
        let Some(column) = &self.column else {
            return;
        };
        let Some(ord) = column.term_ords(doc).next() else {
            return;
        };
        self.buffer.clear();
        if !matches!(column.ord_to_bytes(ord, &mut self.buffer), Ok(true)) {
            return;
        }
        match vector::dot_quantised(&self.query, &self.buffer) {
            Ok(score) if score < self.min_similarity => {}
            Ok(score) => {
                self.heap.push(ScoredAddress {
                    score,
                    address: DocAddress::new(self.segment, doc),
                });
                if self.heap.len() > self.limit {
                    let _ = self.heap.pop();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn harvest(self) -> Vec<ScoredAddress> {
        if let Some(error) = self.error {
            tracing::warn!(%error, "skipped stored vectors of the wrong shape");
        }
        self.heap.into_vec()
    }
}

#[cfg(test)]
mod tests;
