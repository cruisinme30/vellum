// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The one search port (D23): every engine implements [`SearchBackend`].
//!
//! A backend stores blocks and title records, ranks lexically (BM25 or the
//! engine's equivalent) and, when given vectors, by cosine similarity. It
//! knows nothing about people: the service hands it a [`Scope`] already
//! narrowed to readable spaces, and checks every result again afterwards.

use std::collections::HashSet;

use async_trait::async_trait;
use vellum_kernel::{BlockId, DocId, SpaceId};

use crate::error::Result;
use crate::model::{Filters, IndexDocument, StoredBlock, TitleHit};

/// A document to write, with one embedding per block when hybrid is on.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    /// The document.
    pub doc: IndexDocument,
    /// One unit-length vector per block, in block order, or `None`.
    pub vectors: Option<Vec<Vec<f32>>>,
}

/// One change in a batch.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Insert or replace a document and all its blocks.
    Upsert(IndexEntry),
    /// Remove a document and all its blocks.
    Remove(DocId),
}

/// Where a query may look.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    /// The spaces to search; never empty (the service returns early).
    pub spaces: &'a [SpaceId],
    /// When set, only these documents (the starred filter).
    pub docs: Option<&'a HashSet<DocId>>,
    /// The remaining drive filters.
    pub filters: &'a Filters,
}

/// A search engine behind the search service.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// A short name for spans and metrics: `tantivy`, `postgres`, `meilisearch`.
    fn name(&self) -> &'static str;

    /// Applies a batch atomically where the engine allows, and makes it
    /// visible to the next query (read-your-writes).
    async fn apply(&self, changes: &[Change]) -> Result<()>;

    /// Removes everything, before a rebuild from the files.
    async fn clear(&self) -> Result<()>;

    /// Blocks matching `text` in scope, best first.
    async fn lexical(&self, text: &str, scope: Scope<'_>, limit: usize)
    -> Result<Vec<StoredBlock>>;

    /// Blocks nearest to `vector` in scope with cosine similarity of at
    /// least `min_similarity`, best first; empty when no vectors are stored.
    async fn semantic(
        &self,
        vector: &[f32],
        min_similarity: f32,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>>;

    /// Documents whose title matches `text`, the last word as a prefix.
    async fn titles(&self, text: &str, scope: Scope<'_>, limit: usize) -> Result<Vec<TitleHit>>;

    /// A stored block.
    async fn block(&self, doc: DocId, block: BlockId) -> Result<Option<StoredBlock>>;

    /// Every block of a document's section, in reading order. `None` is the
    /// part before the first heading.
    async fn section(&self, doc: DocId, section: Option<BlockId>) -> Result<Vec<StoredBlock>>;
}
