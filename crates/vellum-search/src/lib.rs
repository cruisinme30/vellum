// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Search and retrieval for Vellum.
//!
//! One port, [`SearchBackend`], with three engines behind it (D23): embedded
//! tantivy for the single binary, Postgres full-text search with `pg_trgm`
//! for HA, and an optional Meilisearch adapter for very large shards. The
//! [`SearchService`] on top adds what every engine shares:
//!
//! - **Block-level retrieval** (D62): hits are blocks with stable IDs and
//!   citations; [`SearchService::context`] returns a hit's heading section.
//! - **Hybrid ranking**: BM25 plus optional local embeddings
//!   ([`Embedder`], [`OnnxEmbedder`]), fused by reciprocal rank.
//! - **Permission filtering** (D61): a readable-spaces pre-filter and an
//!   exact batch check through [`ReadAccess`], deny by default.
//! - **Drive filters** (R-DR-9): owner, modified range, type, location,
//!   starred, trash.
//! - **Incremental indexing** on every save, and full rebuilds, through the
//!   [`Indexer`].
//! - **Encryption at rest** (D63): the tantivy index is sealed with
//!   XChaCha20-Poly1305 chunk by chunk.

pub mod access;
pub mod api;
mod authz;
pub mod backend;
#[doc(hidden)]
pub mod conformance;
pub mod error;
pub mod fusion;
pub mod indexer;
pub mod meili_index;
pub mod model;
pub mod onnx;
pub mod postgres_index;
pub mod service;
mod stars;
pub mod tantivy_index;
pub mod telemetry;
pub mod text;
pub mod vector;

pub use access::{Principal, ReadAccess, Stars};
pub use api::SearchApi;
pub use backend::{Change, IndexEntry, Scope, SearchBackend};
pub use error::{Result, SearchError};
pub use indexer::{DocumentSource, Indexer, IndexerConfig, rebuild};
pub use meili_index::{MeiliBackend, MeiliConfig};
pub use model::{
    BlockKind, DocKind, Filters, Fragment, IndexBlock, IndexDocument, SearchHit, SearchPage,
    SearchQuery, StoredBlock, TitleHit,
};
pub use onnx::{OnnxConfig, OnnxEmbedder};
pub use postgres_index::PostgresBackend;
pub use service::{Context, SearchService, Tuning, Update};
pub use tantivy_index::{IndexKey, Location, Opened, TantivyBackend, TantivyConfig};
pub use vector::Embedder;
