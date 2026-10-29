// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The storage port: the [`BlobStore`] trait and the types it speaks.

use std::{fmt, time::SystemTime};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

use crate::{error::StoreResult, path::BlobPath};

/// Opaque version tag of a blob's current content.
///
/// Each adapter chooses its own format (a content hash for local folders,
/// the object `ETag` for S3, a revision id for Google Drive). Two etags from
/// the same adapter are equal exactly when the content is the same version.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Etag(String);

impl Etag {
    /// Wraps an adapter-specific version string.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The adapter-specific version string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Etag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Etag({})", self.0)
    }
}

impl fmt::Display for Etag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Metadata of a stored blob, including its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobMeta {
    /// Where the blob lives.
    pub path: BlobPath,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time reported by the backend.
    pub modified: SystemTime,
    /// Version of the content.
    pub etag: Etag,
}

/// A blob's content together with its metadata.
#[derive(Clone, Debug)]
pub struct Blob {
    /// Raw bytes.
    pub bytes: Bytes,
    /// Metadata, with the etag matching `bytes`.
    pub meta: BlobMeta,
}

/// A listing entry. Listings skip etags because computing them can cost a
/// full read per blob; call [`BlobStore::head`] when the version matters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobEntry {
    /// Where the blob lives.
    pub path: BlobPath,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time reported by the backend.
    pub modified: SystemTime,
}

/// Condition a write or delete must satisfy to take effect.
///
/// This is how Vellum avoids last-write-wins: a writer states which version
/// it based its change on, and the store refuses the write if the blob has
/// moved on, sending the caller to the three-way merge instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Precondition {
    /// Apply unconditionally.
    #[default]
    None,
    /// Apply only if no blob exists at the path.
    IfAbsent,
    /// Apply only if the current blob has this etag.
    IfMatch(Etag),
}

/// The storage port every backend implements (local folder, S3, Google
/// Drive, Dropbox, `OneDrive`, Git) and every decorator wraps (encryption).
///
/// All adapters must pass `conformance` (feature `conformance`). Writes are atomic: a
/// reader sees the old content or the new content, never a mix.
#[async_trait]
pub trait BlobStore: Send + Sync + fmt::Debug {
    /// Short backend name for spans and metrics, such as `"local"`.
    fn backend(&self) -> &'static str;

    /// Reads a blob and its current version.
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob>;

    /// Reads a blob's metadata and current version.
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta>;

    /// Writes a blob atomically if `condition` holds, creating parent
    /// directories as needed, and returns the new metadata.
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta>;

    /// Removes a blob if `condition` holds. With `IfAbsent` the delete
    /// fails either way: `AlreadyExists` or `NotFound`.
    async fn delete(&self, path: &BlobPath, condition: Precondition) -> StoreResult<()>;

    /// Moves a blob, failing with `AlreadyExists` if `to` is taken.
    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta>;

    /// Streams every blob at or beneath `dir` (the whole store for `None`),
    /// recursively and in no particular order.
    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>>;
}
