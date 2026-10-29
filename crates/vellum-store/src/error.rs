// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors returned by every [`crate::BlobStore`] adapter.

use crate::{Etag, path::BlobPath};

/// A storage failure. Adapters map their native errors onto these variants
/// so callers (merge, watcher, encryption) can react the same way to every
/// backend.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// No blob exists at the path.
    #[error("blob not found: {0}")]
    NotFound(BlobPath),

    /// A blob already exists where none was expected.
    #[error("blob already exists: {0}")]
    AlreadyExists(BlobPath),

    /// The blob changed since the caller last read it. `current` is the
    /// etag now stored, or `None` when the blob has since been removed.
    #[error("precondition failed for {path}: blob changed")]
    PreconditionFailed {
        /// Path of the blob.
        path: BlobPath,
        /// Etag currently stored, if any.
        current: Option<Etag>,
    },

    /// The path breaks [`BlobPath`] rules or resolves outside the store.
    #[error("invalid path {path:?}: {reason}")]
    InvalidPath {
        /// The rejected path.
        path: String,
        /// Which rule it broke.
        reason: &'static str,
    },

    /// An operating-system I/O error.
    #[error("i/o error on {path}: {source}")]
    Io {
        /// Path being accessed.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Any other backend failure (network, auth, quota, ...).
    #[error("backend error: {0}")]
    Backend(String),
}

impl StoreError {
    /// Short, stable label for metrics and span fields.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::AlreadyExists(_) => "already_exists",
            Self::PreconditionFailed { .. } => "precondition_failed",
            Self::InvalidPath { .. } => "invalid_path",
            Self::Io { .. } => "io",
            Self::Backend(_) => "backend",
        }
    }
}

/// Shorthand for results from this crate.
pub type StoreResult<T> = Result<T, StoreError>;
