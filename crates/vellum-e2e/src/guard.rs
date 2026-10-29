// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps readable content out of private spaces (D208).
//!
//! [`Guard`] wraps the workspace's blob store. Every write into a private
//! space's folder must be a recipient envelope, whichever service makes it
//! (drive, live editing, sync, books, imports). Structural files that
//! carry no page content stay writable: folder manifests, sibling order
//! and shortcuts.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use vellum_crypto::recipients::is_recipient_envelope;
use vellum_store::layout::{FOLDER_META, SYSTEM_DIR};
use vellum_store::{
    Blob, BlobEntry, BlobMeta, BlobPath, BlobStore, Precondition, StoreError, StoreResult,
};

/// The sibling-order file the drive keeps in each folder.
const ORDER_FILE: &str = ".vellum-order.yaml";

/// Why a write was refused.
const REFUSED: &str =
    "this space is end-to-end encrypted: only pages sealed on a member's device can be saved here";

/// True for files inside a space that hold no page content, which a
/// private space keeps readable: folder manifests, sibling order,
/// shortcuts and Vellum's own state. `rel` is relative to the space.
#[must_use]
pub fn structural(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name == FOLDER_META
        || name == ORDER_FILE
        || name.to_ascii_lowercase().ends_with(".shortcut.yaml")
        || rel.split('/').any(|s| s == SYSTEM_DIR)
}

/// The folders of the private spaces this server knows about, shared by
/// the [`Guard`] and the service that changes key lists.
#[derive(Clone, Default)]
pub struct PrivateRoots(Arc<RwLock<BTreeSet<BlobPath>>>);

impl fmt::Debug for PrivateRoots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PrivateRoots").field(&self.len()).finish()
    }
}

impl PrivateRoots {
    /// Marks `root` private.
    pub fn insert(&self, root: BlobPath) {
        let _ = self
            .0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(root);
    }

    /// Replaces the whole set.
    pub fn replace(&self, roots: BTreeSet<BlobPath>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = roots;
    }

    /// How many spaces are private.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// True when no space is private.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `path` relative to the private space it is in, if any.
    #[must_use]
    pub fn relative(&self, path: &BlobPath) -> Option<String> {
        let roots = self.0.read().unwrap_or_else(PoisonError::into_inner);
        roots.iter().find_map(|root| {
            let rest = path.as_str().strip_prefix(root.as_str())?;
            rest.strip_prefix('/').map(str::to_owned)
        })
    }

    /// True when a write of `bytes` to `path` would put readable content
    /// into a private space.
    fn refuses(&self, path: &BlobPath, bytes: &[u8]) -> bool {
        self.relative(path)
            .is_some_and(|rel| !structural(&rel) && !is_recipient_envelope(bytes))
    }
}

/// A blob store that refuses readable writes into private spaces.
pub struct Guard {
    inner: Arc<dyn BlobStore>,
    roots: PrivateRoots,
}

impl fmt::Debug for Guard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guard")
            .field("inner", &self.inner.backend())
            .field("roots", &self.roots)
            .finish()
    }
}

impl Guard {
    /// Guards `inner` with the private spaces in `roots`.
    #[must_use]
    pub fn new(inner: Arc<dyn BlobStore>, roots: PrivateRoots) -> Self {
        Self { inner, roots }
    }

    fn refused(path: &BlobPath) -> StoreError {
        tracing::warn!(
            path = %path,
            monotonic_counter.vellum.e2e.plaintext_refused = 1_u64,
            "refused a readable write into a private space"
        );
        StoreError::InvalidPath {
            path: path.to_string(),
            reason: REFUSED,
        }
    }
}

#[async_trait]
impl BlobStore for Guard {
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
        if self.roots.refuses(path, &bytes) {
            return Err(Self::refused(path));
        }
        self.inner.put(path, bytes, condition).await
    }

    async fn delete(&self, path: &BlobPath, condition: Precondition) -> StoreResult<()> {
        self.inner.delete(path, condition).await
    }

    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        // Moving a readable page into a private space is a write too.
        if self.roots.relative(to).is_some_and(|rel| !structural(&rel)) {
            let moving = self.inner.get(from).await?;
            if self.roots.refuses(to, &moving.bytes) {
                return Err(Self::refused(to));
            }
        }
        self.inner.rename(from, to).await
    }

    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>> {
        self.inner.list(dir)
    }
}
