// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum's index database.
//!
//! Files on disk are the source of truth; this crate holds the rebuildable
//! index over them (spaces, folders, documents, the `page_ancestors` closure)
//! plus the database-owned tables `grants`, `user_doc_state` and
//! `collab_updates` (the live-editing change log, written by `vellum-collab`).
//!
//! Two backends share one schema shape:
//! - **SQLite** for the single binary, encrypted with SQLCipher when a key is given.
//! - **Postgres** for the HA profiles.
//!
//! Conventions: IDs are ULID text, timestamps are UTC epoch milliseconds,
//! and `space_id` is the shard key on every table. Migrations are forward-only.

mod config;
mod error;
mod index;
mod reindex;
mod telemetry;

pub use config::{IndexConfig, IndexKey};
pub use error::{Error, Result};
pub use index::{Backend, Index, IndexPool};
pub use reindex::{FileReader, PlainFiles, Problem, ProblemKind, ReindexReport};
pub use telemetry::{RowCount, traced};

/// The sqlx version this crate is built on, so dependants share one copy.
pub use sqlx;
