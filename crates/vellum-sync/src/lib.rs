// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sync service behind the local-first clients (ADR 0023 D98, ADR 0029).
//!
//! Clients push named mutations, each applied at most once per client and
//! in order, and pull the record changes they may read since a cursor.
//! Services register the server half of each mutator and publish the
//! changes they make through other paths, so every client converges.
//!
//! - [`domain`]: the wire contract and log records.
//! - [`ports`]: the store, read permissions and mutators this crate needs.
//! - [`Sync`]: the push, pull and publish use cases.
//! - [`memory`]: the in-memory store; [`conformance`]: the adapter suite.

pub mod app;
pub mod conformance;
pub mod cursor;
pub mod domain;
pub mod memory;
pub mod ports;

pub use app::{MAX_PUSH, Mutators, PULL_PAGE, Sync};
pub use domain::{
    Change, ClientId, ClientMutation, Patch, PullRequest, PullResponse, PushRequest, PushResponse,
    Record, Rejected,
};
pub use ports::{
    AccessKey, ClientState, LogInfo, MutationContext, ReadAccess, ServerMutator, SyncStore,
};
