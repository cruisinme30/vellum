// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum's index database.
//!
//! Files on disk are the source of truth; this crate holds the rebuildable
//! index over them (spaces, folders, documents, the `page_ancestors` closure)
//! plus the database-owned tables `grants`, `user_doc_state`,
//! `collab_updates` (the live-editing change log, written by `vellum-collab`)
//! and `identity_records` (accounts and sessions shared by replicas, written
//! by `vellum-identity`), the replica tables in [`cluster`] and [`feed`], the
//! job queue in [`jobs`], shared rate-limit counts in [`rate`], and the
//! drive's page change log in [`changes`].
//!
//! Two backends share one schema shape:
//! - **SQLite** for the single binary, encrypted with SQLCipher when a key is given.
//! - **Postgres** for the HA profiles.
//!
//! Conventions: IDs are ULID text, timestamps are UTC epoch milliseconds,
//! and `space_id` is the shard key on every table. Migrations are forward-only.

pub mod changes;
pub mod cluster;
mod config;
mod error;
pub mod feed;
mod index;
pub mod jobs;
pub mod rate;
mod reindex;
mod router;
mod shard;
mod telemetry;

pub use config::{IndexConfig, IndexKey};
pub use error::{Error, Result};
pub use index::{Backend, Index, IndexPool};
pub use reindex::{FileReader, Listing, PlainFiles, Problem, ProblemKind, ReindexReport, Tree};
pub use router::ShardRouter;
pub use shard::{MAIN_URL_VAR, SHARDS_VAR, Shard, ShardedIndex, ShardsConfig, shard_url_var};
pub use telemetry::{RowCount, traced};

/// The sqlx version this crate is built on, so dependants share one copy.
pub use sqlx;
