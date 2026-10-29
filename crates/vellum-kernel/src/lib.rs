// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Shared foundations for every Vellum crate.
//!
//! The kernel sits at the bottom of the crate graph and depends on no other
//! Vellum crate. It carries the observability baseline ([`telemetry`]) and
//! typed object IDs ([`id`])
//! block IDs ([`block`]) and share-link tokens ([`share`]).

mod base32;
pub mod block;
pub mod id;
pub mod share;
pub mod telemetry;

pub use block::BlockId;
pub use id::{
    BlobId, BookId, CollectionId, CommentId, DocId, DriveId, EventId, FolderId, IdParseError,
    IssueId, ObjectId, RequestId, ShareId, SpaceId, TaskId, TeamId, UserId, VersionId,
};
pub use share::{ShareToken, ShareTokenError, ShareTokenHash};
