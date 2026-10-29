// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive: Vellum's organisation layer (brief §6, R-DR-1 to R-DR-10).
//!
//! What you see is the disk. Spaces (shared drives and My Drives) are
//! directories, folders are directories with a `.vellum-folder.yaml`,
//! pages are Markdown files whose front-matter `id` is their only identity.
//! [`Drive`] is the service: create, save (three-way merged, never
//! last-write-wins), rename with link rewriting, move, copy with fresh IDs,
//! shortcuts, a 30-day trash with exact restore, stars, recents, activity,
//! backlinks, quotas and version retention. It works through three ports:
//! the [`vellum_store::BlobStore`] for files, a [`DriveIndex`] for the
//! rebuildable index, and [`Access`] for authorization.

pub mod authz;
mod drive;
pub mod error;
pub mod files;
pub mod frontmatter;
pub mod layout;
pub mod links;
pub mod memory;
pub mod model;
pub mod ports;
pub mod retention;
pub mod sql;

pub use drive::{Drive, Filter, OP_SECONDS, OPS_TOTAL, SaveOutcome, Saved};
pub use error::{DriveError, DriveResult};
pub use model::{
    Activity, ActivityKind, Node, NodeKind, RetentionPolicy, ScanReport, Space, SpaceKind,
    SpaceSettings, TrashState, Usage, VersionRecord,
};
pub use ports::{Access, Clock, DriveIndex, Permission, SystemClock};
