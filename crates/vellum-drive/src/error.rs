// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors returned by the drive.

use vellum_kernel::{ErrorKind, ObjectId, SpaceId};
use vellum_store::StoreError;

/// A drive failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DriveError {
    /// No such space, or the caller may not know it exists.
    #[error("space not found: {0}")]
    SpaceNotFound(SpaceId),

    /// No such node, or the caller may not know it exists.
    #[error("not found: {0}")]
    NotFound(ObjectId),

    /// The caller lacks the permission for this operation.
    #[error("permission denied")]
    PermissionDenied,

    /// The request is malformed (bad name, wrong node kind, move into itself).
    #[error("invalid request: {0}")]
    Invalid(String),

    /// A [`ContentCheck`](crate::ContentCheck) refused the page text.
    #[error("{0}")]
    Refused(String),

    /// Something with that identity or name already exists.
    #[error("already exists: {0}")]
    AlreadyExists(String),

    /// The write would take the space over its storage quota.
    #[error("storage quota exceeded: {used} of {quota} bytes used, {needed} more needed")]
    QuotaExceeded {
        /// Bytes in use now.
        used: u64,
        /// The space's quota in bytes.
        quota: u64,
        /// Bytes the write would add.
        needed: u64,
    },

    /// A file's front matter or manifest cannot be read.
    #[error("malformed file {path}: {reason}")]
    Malformed {
        /// The file.
        path: String,
        /// What is wrong with it.
        reason: String,
    },

    /// The blob store failed.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The index adapter failed.
    #[error("index error: {0}")]
    Index(String),

    /// The authorization adapter failed.
    #[error("authorization error: {0}")]
    Access(String),
}

impl DriveError {
    /// Short, stable label for metrics and span fields.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SpaceNotFound(_) | Self::NotFound(_) => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::Invalid(_) => "invalid",
            Self::Refused(_) => "refused",
            Self::AlreadyExists(_) => "already_exists",
            Self::QuotaExceeded { .. } => "quota_exceeded",
            Self::Malformed { .. } => "malformed",
            Self::Store(_) => "store",
            Self::Index(_) => "index",
            Self::Access(_) => "access",
        }
    }

    /// The kernel error kind, which decides the HTTP status and retry policy.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::SpaceNotFound(_) | Self::NotFound(_) | Self::Store(StoreError::NotFound(_)) => {
                ErrorKind::NotFound
            }
            Self::PermissionDenied => ErrorKind::PermissionDenied,
            Self::Invalid(_)
            | Self::Refused(_)
            | Self::Malformed { .. }
            | Self::Store(StoreError::InvalidPath { .. }) => ErrorKind::InvalidInput,
            Self::AlreadyExists(_)
            | Self::Store(StoreError::AlreadyExists(_) | StoreError::PreconditionFailed { .. }) => {
                ErrorKind::Conflict
            }
            Self::QuotaExceeded { .. } => ErrorKind::RateLimited,
            Self::Store(_) | Self::Index(_) | Self::Access(_) => ErrorKind::Unavailable,
        }
    }
}

impl From<DriveError> for vellum_kernel::Error {
    fn from(err: DriveError) -> Self {
        let kind = err.kind();
        let message = match &err {
            // Internal detail stays in the source chain, out of the message.
            DriveError::Store(_) | DriveError::Index(_) | DriveError::Access(_) => {
                "storage is unavailable".to_owned()
            }
            other => other.to_string(),
        };
        Self::new(kind, message).with_source(err)
    }
}

/// Shorthand for results from this crate.
pub type DriveResult<T> = Result<T, DriveError>;
