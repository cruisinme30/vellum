// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors from search, mapped onto the kernel's error kinds at the edge.

use vellum_kernel::{Error, ErrorKind};

/// Anything that can go wrong while indexing or searching.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SearchError {
    /// The local tantivy index failed.
    #[error("tantivy index: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    /// The tantivy query could not be built.
    #[error("tantivy query: {0}")]
    Query(#[from] tantivy::query::QueryParserError),
    /// The Postgres index failed.
    #[error("postgres index: {0}")]
    Postgres(#[from] sqlx::Error),
    /// The Postgres schema migration failed.
    #[error("postgres migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The Meilisearch server failed or answered with an error.
    #[error("meilisearch: {0}")]
    Meilisearch(String),
    /// The embedding model failed.
    #[error("embedding model: {0}")]
    Embedding(String),
    /// A stored record could not be read back.
    #[error("corrupt index record: {0}")]
    Corrupt(String),
    /// Encryption or decryption of the index failed.
    #[error("index encryption: {0}")]
    Crypto(&'static str),
    /// The permission service could not answer.
    #[error("authorization: {0}")]
    Access(String),
    /// The document source could not load a document.
    #[error("document source: {0}")]
    Source(String),
    /// The request was invalid.
    #[error("invalid request: {0}")]
    Invalid(String),
    /// A file-system operation failed.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

impl SearchError {
    /// The kernel error kind this maps to.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Invalid(_) | Self::Query(_) => ErrorKind::InvalidInput,
            Self::Meilisearch(_) | Self::Postgres(_) | Self::Access(_) | Self::Source(_) => {
                ErrorKind::Unavailable
            }
            Self::Tantivy(_)
            | Self::Migrate(_)
            | Self::Embedding(_)
            | Self::Corrupt(_)
            | Self::Crypto(_)
            | Self::Io(_) => ErrorKind::Internal,
        }
    }
}

impl From<SearchError> for Error {
    fn from(error: SearchError) -> Self {
        // Only invalid-input messages are written for the caller; the rest
        // can name internals, so they travel as the source for logs only.
        let kind = error.kind();
        let message = match &error {
            SearchError::Invalid(message) => format!("invalid search: {message}"),
            SearchError::Query(_) => "the search query could not be parsed".to_owned(),
            _ if kind == ErrorKind::Unavailable => "search is unavailable".to_owned(),
            _ => "search failed".to_owned(),
        };
        Self::new(kind, message).with_source(error)
    }
}

/// Shorthand for search results.
pub type Result<T, E = SearchError> = std::result::Result<T, E>;
