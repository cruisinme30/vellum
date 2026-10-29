// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The search service: permission-filtered, hybrid, block-level search.
//!
//! A query runs in four steps:
//!
//! 1. **Scope.** The principal's readable spaces (from authorization),
//!    intersected with the requested spaces, plus starred documents when
//!    asked. Nothing readable means an empty answer without touching the
//!    index.
//! 2. **Rank.** BM25 over the scope and, when an embedder is configured,
//!    nearest neighbours of the query vector; fused by reciprocal rank.
//! 3. **Check.** Every candidate document passes one exact batch check.
//!    Candidates widen until a page is full or the index runs out.
//! 4. **Present.** Snippets with highlights, heading paths and citations.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{Instrument as _, field};
use vellum_kernel::{BlockId, DocId};

use crate::access::{Principal, ReadAccess, Stars};
use crate::backend::{Change, IndexEntry, Scope, SearchBackend};
use crate::error::{Result, SearchError};
use crate::fusion::{Ranked, fuse};
use crate::model::{
    Filters, IndexDocument, MAX_LIMIT, SearchHit, SearchPage, SearchQuery, StoredBlock, TitleHit,
    citation,
};
use crate::telemetry::traced;
use crate::text;
use crate::vector::{self, Embedder};

/// The deepest offset a query may page to.
pub const MAX_OFFSET: usize = 1_000;

/// The longest query accepted, in bytes.
pub const MAX_QUERY_BYTES: usize = 1_024;

/// How many passages are embedded per model call.
const EMBED_BATCH: usize = 32;

/// Tuning knobs, with defaults that suit every profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// Weight of the BM25 list in fusion.
    pub lexical_weight: f64,
    /// Weight of the vector list in fusion.
    pub semantic_weight: f64,
    /// The fewest candidates fetched per list.
    pub min_candidates: usize,
    /// The most candidates fetched per list.
    pub max_candidates: usize,
    /// The lowest cosine similarity a semantic hit needs. Nearest
    /// neighbours always exist, so without a floor every block would match.
    pub min_similarity: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            lexical_weight: 1.0,
            semantic_weight: 1.0,
            min_candidates: 50,
            max_candidates: 2_000,
            min_similarity: 0.35,
        }
    }
}

/// One write to the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// Index or re-index a document.
    Upsert(IndexDocument),
    /// Drop a document.
    Remove(DocId),
}

/// A block with its whole section: what "ask this space" and MCP quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// The block asked for.
    pub block: StoredBlock,
    /// Every block of its section, in reading order.
    pub section: Vec<StoredBlock>,
    /// The citation of the block.
    pub citation: String,
}

/// The search service.
#[derive(Clone)]
pub struct SearchService {
    backend: Arc<dyn SearchBackend>,
    access: Arc<dyn ReadAccess>,
    stars: Arc<dyn Stars>,
    embedder: Option<Arc<dyn Embedder>>,
    tuning: Tuning,
}

impl std::fmt::Debug for SearchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchService")
            .field("backend", &self.backend.name())
            .field(
                "embedder",
                &self.embedder.as_ref().map(|e| e.model_id().to_owned()),
            )
            .field("tuning", &self.tuning)
            .finish_non_exhaustive()
    }
}

type Key = (DocId, BlockId);

impl SearchService {
    /// A lexical-only service over `backend`.
    #[must_use]
    pub fn new(
        backend: Arc<dyn SearchBackend>,
        access: Arc<dyn ReadAccess>,
        stars: Arc<dyn Stars>,
    ) -> Self {
        Self {
            backend,
            access,
            stars,
            embedder: None,
            tuning: Tuning::default(),
        }
    }

    /// Turns on hybrid ranking with a local embedding model.
    #[must_use]
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Replaces the tuning knobs.
    #[must_use]
    pub const fn with_tuning(mut self, tuning: Tuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// The backend's name.
    #[must_use]
    pub fn engine(&self) -> &'static str {
        self.backend.name()
    }

    /// Writes a batch to the index; visible to the next query.
    ///
    /// # Errors
    /// Embedding or backend failures; nothing is written then.
    pub async fn write(&self, updates: Vec<Update>) -> Result<()> {
        let docs: Vec<IndexDocument> = updates
            .iter()
            .filter_map(|u| match u {
                Update::Upsert(doc) => Some(doc.clone()),
                Update::Remove(_) => None,
            })
            .collect();
        let mut vectors = self.embed_documents(&docs).await?.into_iter();
        let mut changes = Vec::with_capacity(updates.len());
        for update in updates {
            changes.push(match update {
                Update::Upsert(doc) => Change::Upsert(IndexEntry {
                    doc,
                    vectors: vectors.next().flatten(),
                }),
                Update::Remove(doc) => Change::Remove(doc),
            });
        }
        let engine = self.backend.name();
        traced(engine, "apply", self.backend.apply(&changes)).await
    }

    /// Empties the index before a rebuild.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn clear(&self) -> Result<()> {
        traced(self.backend.name(), "clear", self.backend.clear()).await
    }

    /// One vector list per document (or `None` each without an embedder).
    async fn embed_documents(&self, docs: &[IndexDocument]) -> Result<Vec<Option<Vec<Vec<f32>>>>> {
        let Some(embedder) = &self.embedder else {
            return Ok(vec![None; docs.len()]);
        };
        let passages: Vec<String> = docs
            .iter()
            .flat_map(|doc| doc.blocks.iter().map(move |block| passage(doc, block)))
            .collect();
        let mut flat = Vec::with_capacity(passages.len());
        for batch in passages.chunks(EMBED_BATCH) {
            let vectors =
                traced("embedder", "embed_passages", embedder.embed_passages(batch)).await?;
            vector::check_shape(&vectors, batch.len(), embedder.dimensions())?;
            flat.extend(vectors);
        }
        let mut flat = flat.into_iter();
        Ok(docs
            .iter()
            .map(|doc| Some(flat.by_ref().take(doc.blocks.len()).collect()))
            .collect())
    }

    /// The spaces a principal may search, narrowed by the request.
    async fn spaces(
        &self,
        principal: &Principal,
        filters: &Filters,
    ) -> Result<Vec<vellum_kernel::SpaceId>> {
        let readable = self.access.readable_spaces(principal).await?;
        if filters.spaces.is_empty() {
            return Ok(readable);
        }
        let wanted: HashSet<_> = filters.spaces.iter().collect();
        Ok(readable
            .into_iter()
            .filter(|s| wanted.contains(s))
            .collect())
    }

    /// The starred documents when the filter asks for them.
    async fn starred(
        &self,
        principal: &Principal,
        filters: &Filters,
    ) -> Result<Option<HashSet<DocId>>> {
        if !filters.starred {
            return Ok(None);
        }
        match principal.user() {
            Some(user) => Ok(Some(self.stars.starred(user).await?)),
            // A share-link visitor has no stars.
            None => Ok(Some(HashSet::new())),
        }
    }

    /// Full-text search: block-level hits, best first, permission-filtered.
    ///
    /// # Errors
    /// [`SearchError::Invalid`] for an empty, overlong or too-deep query;
    /// authorization or backend failures otherwise.
    pub async fn search(&self, principal: &Principal, query: &SearchQuery) -> Result<SearchPage> {
        let span = tracing::info_span!(
            "search.query",
            search.engine = self.backend.name(),
            search.hybrid = field::Empty,
            search.candidates = field::Empty,
            search.hits = field::Empty,
        );
        let start = Instant::now();
        let out = self
            .search_inner(principal, query)
            .instrument(span.clone())
            .await;
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        if let Ok(page) = &out {
            let _ = span.record("search.hybrid", page.hybrid);
            let _ = span.record("search.hits", page.hits.len());
        }
        tracing::debug!(
            histogram.vellum.search.query_ms = ms,
            search.engine = self.backend.name()
        );
        out
    }

    async fn search_inner(&self, principal: &Principal, query: &SearchQuery) -> Result<SearchPage> {
        let text = validate(&query.text, query.offset)?;
        let limit = query.limit.clamp(1, MAX_LIMIT);
        let empty = SearchPage {
            hits: Vec::new(),
            has_more: false,
            hybrid: false,
        };
        let spaces = self.spaces(principal, &query.filters).await?;
        if spaces.is_empty() {
            return Ok(empty);
        }
        let starred = self.starred(principal, &query.filters).await?;
        if starred.as_ref().is_some_and(HashSet::is_empty) {
            return Ok(empty);
        }
        let scope = Scope {
            spaces: &spaces,
            docs: starred.as_ref(),
            filters: &query.filters,
        };

        let query_vector = self.embed_query(text).await;
        let hybrid = query_vector.is_some();
        let need = query.offset + limit + 1;
        let mut candidates =
            (need * 2).clamp(self.tuning.min_candidates, self.tuning.max_candidates);
        let engine = self.backend.name();
        let (visible, blocks) = loop {
            let lexical = traced(
                engine,
                "lexical",
                self.backend.lexical(text, scope, candidates),
            )
            .await?;
            let semantic = match &query_vector {
                Some(v) => {
                    traced(
                        engine,
                        "semantic",
                        self.backend
                            .semantic(v, self.tuning.min_similarity, scope, candidates),
                    )
                    .await?
                }
                None => Vec::new(),
            };
            let exhausted = lexical.len() < candidates && semantic.len() < candidates;
            let (visible, blocks) = self.fuse_and_check(principal, lexical, semantic).await?;
            if visible.len() >= need || exhausted || candidates >= self.tuning.max_candidates {
                break (visible, blocks);
            }
            candidates = (candidates * 4).min(self.tuning.max_candidates);
        };
        let _ = tracing::Span::current().record("search.candidates", candidates);

        let terms = text::query_terms(text);
        let has_more = visible.len() > query.offset + limit;
        let hits = visible
            .into_iter()
            .skip(query.offset)
            .take(limit)
            .filter_map(|(key, score)| blocks.get(&key).map(|block| hit(block, score, &terms)))
            .collect();
        Ok(SearchPage {
            hits,
            has_more,
            hybrid,
        })
    }

    /// The query vector, or `None` without an embedder or when it fails
    /// (search degrades to BM25 rather than failing).
    async fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        let embedder = self.embedder.as_ref()?;
        match traced("embedder", "embed_query", embedder.embed_query(text)).await {
            Ok(mut v) if v.len() == embedder.dimensions() => {
                vector::normalise(&mut v);
                Some(v)
            }
            Ok(_) => {
                tracing::warn!("embedder returned the wrong dimensions; using BM25 only");
                None
            }
            Err(error) => {
                tracing::warn!(%error, "query embedding failed; using BM25 only");
                None
            }
        }
    }

    async fn fuse_and_check(
        &self,
        principal: &Principal,
        lexical: Vec<StoredBlock>,
        semantic: Vec<StoredBlock>,
    ) -> Result<(Vec<(Key, f64)>, HashMap<Key, StoredBlock>)> {
        let key = |b: &StoredBlock| (b.doc, b.block);
        let lists = [
            Ranked {
                items: lexical.iter().map(key).collect(),
                weight: self.tuning.lexical_weight,
            },
            Ranked {
                items: semantic.iter().map(key).collect(),
                weight: self.tuning.semantic_weight,
            },
        ];
        let fused = fuse(&lists);
        let mut blocks: HashMap<Key, StoredBlock> = HashMap::with_capacity(fused.len());
        for block in lexical.into_iter().chain(semantic) {
            let _ = blocks.entry(key(&block)).or_insert(block);
        }
        let mut docs: Vec<DocId> = fused.iter().map(|((doc, _), _)| *doc).collect();
        docs.sort_unstable();
        docs.dedup();
        let allowed = if docs.is_empty() {
            HashSet::new()
        } else {
            self.access.filter_readable(principal, &docs).await?
        };
        let visible = fused
            .into_iter()
            .filter(|((doc, _), _)| allowed.contains(doc))
            .collect();
        Ok((visible, blocks))
    }

    /// Instant title search (⌘K): the last word matches as a prefix.
    ///
    /// # Errors
    /// Authorization or backend failures.
    pub async fn titles(
        &self,
        principal: &Principal,
        text: &str,
        filters: &Filters,
        limit: usize,
    ) -> Result<Vec<TitleHit>> {
        let text = validate(text, 0)?;
        let limit = limit.clamp(1, MAX_LIMIT);
        let spaces = self.spaces(principal, filters).await?;
        if spaces.is_empty() {
            return Ok(Vec::new());
        }
        let starred = self.starred(principal, filters).await?;
        let scope = Scope {
            spaces: &spaces,
            docs: starred.as_ref(),
            filters,
        };
        let engine = self.backend.name();
        let mut fetch = (limit * 3).max(self.tuning.min_candidates);
        loop {
            let found = traced(engine, "titles", self.backend.titles(text, scope, fetch)).await?;
            let exhausted = found.len() < fetch;
            let docs: Vec<DocId> = found.iter().map(|t| t.doc).collect();
            let allowed = if docs.is_empty() {
                HashSet::new()
            } else {
                self.access.filter_readable(principal, &docs).await?
            };
            let visible: Vec<TitleHit> = found
                .into_iter()
                .filter(|t| allowed.contains(&t.doc))
                .collect();
            if visible.len() >= limit || exhausted || fetch >= self.tuning.max_candidates {
                return Ok(visible.into_iter().take(limit).collect());
            }
            fetch = (fetch * 4).min(self.tuning.max_candidates);
        }
    }

    /// A block with its heading section, if the principal can read it.
    ///
    /// Returns `None` both when it does not exist and when it is not
    /// readable, so existence never leaks.
    ///
    /// # Errors
    /// Authorization or backend failures.
    pub async fn context(
        &self,
        principal: &Principal,
        doc: DocId,
        block: BlockId,
    ) -> Result<Option<Context>> {
        let allowed = self.access.filter_readable(principal, &[doc]).await?;
        if !allowed.contains(&doc) {
            return Ok(None);
        }
        let engine = self.backend.name();
        let Some(found) = traced(engine, "block", self.backend.block(doc, block)).await? else {
            return Ok(None);
        };
        let section = traced(engine, "section", self.backend.section(doc, found.section)).await?;
        Ok(Some(Context {
            citation: citation(doc, block),
            block: found,
            section,
        }))
    }
}

/// Trims and checks a query.
fn validate(text: &str, offset: usize) -> Result<&str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(SearchError::Invalid("the query is empty".into()));
    }
    if text.len() > MAX_QUERY_BYTES {
        return Err(SearchError::Invalid(format!(
            "the query is longer than {MAX_QUERY_BYTES} bytes"
        )));
    }
    if offset > MAX_OFFSET {
        return Err(SearchError::Invalid(format!(
            "results deeper than {MAX_OFFSET} are not served; refine the query"
        )));
    }
    Ok(text)
}

/// The passage embedded for a block: title and headings give it context.
fn passage(doc: &IndexDocument, block: &crate::model::IndexBlock) -> String {
    let mut out = doc.title.clone();
    for heading in &block.heading_path {
        out.push_str(" › ");
        out.push_str(heading);
    }
    out.push('\n');
    out.push_str(&block.text);
    out
}

fn hit(block: &StoredBlock, score: f64, terms: &[String]) -> SearchHit {
    SearchHit {
        space: block.space,
        doc: block.doc,
        block: block.block,
        section: block.section,
        doc_kind: block.doc_kind,
        block_kind: block.block_kind,
        title: block.title.clone(),
        path: block.path.clone(),
        heading_path: block.heading_path.clone(),
        snippet: text::snippet(&block.text, terms),
        score,
        citation: citation(block.doc, block.block),
    }
}
