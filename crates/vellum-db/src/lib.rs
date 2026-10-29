// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum's index database.
//!
//! Files on disk are the source of truth; this crate holds the rebuildable
//! index over them (spaces, folders, documents, the `page_ancestors` closure)
//! plus the two database-owned tables, `grants` and `user_doc_state`.
//!
//! The single binary keeps the index in SQLite, encrypted with SQLCipher
//! when a key is given. The Postgres adapter for the HA profiles shares the
//! same schema shape.
//!
//! Conventions: IDs are ULID text, timestamps are UTC epoch milliseconds,
//! and `space_id` is the shard key on every table. Migrations are forward-only.

mod config;
mod error;
mod index;
mod telemetry;

pub use config::{IndexConfig, IndexKey};
pub use error::{Error, Result};
pub use index::{Backend, Index};
