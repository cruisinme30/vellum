// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum's storage layer.
//!
//! [`BlobStore`] is the port every storage backend implements (local
//! folder, S3, Google Drive, Dropbox, `OneDrive`, Git) and every decorator
//! wraps. Blobs are addressed by validated [`BlobPath`]s, versioned by
//! opaque [`Etag`]s, and written under a [`Precondition`] so a stale writer
//! is refused instead of silently overwriting newer content.

mod error;
#[cfg(feature = "git")]
pub mod git;
pub mod layout;
mod local;
mod merge;
pub mod metered;
mod path;
mod reconcile;
#[cfg(feature = "remote")]
pub mod remote;
mod store;
pub mod watch;

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;

pub use error::{StoreError, StoreResult};
pub use local::LocalStore;
pub use merge::{MergeKind, Merged, merge3};
pub use metered::Metered;
pub use path::{BlobPath, MAX_PATH_BYTES, TEMP_PREFIX};
pub use reconcile::{Base, Saved, Writer, conflict_path, save};
pub use store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition};
