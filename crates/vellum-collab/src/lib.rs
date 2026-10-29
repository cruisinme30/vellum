// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Real-time collaboration rooms over the yrs CRDT.
//!
//! A [`Hub`] owns one room per open document. Editors join a room over a
//! WebSocket ([`ws`]) and speak the standard Yjs sync and awareness
//! protocol, so the web editor uses the stock `y-prosemirror` binding.
//!
//! ```text
//!  editor ──ws──▶ Room ──every ≤ 50 ms──▶ UpdateStore (append-only log)
//!                  │ fan-out to peers
//!                  └──30 s idle or Done──▶ MarkdownCodec ──▶ DocumentFiles
//! ```
//!
//! * Updates are applied and fanned out at once, and appended to the
//!   [`UpdateStore`] in batches at most [`CollabConfig::batch_window`] apart.
//! * After [`CollabConfig::idle_version_after`] without edits, or when an
//!   editor presses Done, the room writes a Markdown version through the
//!   [`MarkdownCodec`] (the dialect's canonical serializer) and compacts its
//!   log.
//! * A room is re-created from the log on any node, so a lost node costs a
//!   reconnect; [`placement`] decides which node owns a room.
//! * W3C `traceparent` is accepted on the upgrade request and per message,
//!   so editor telemetry joins the server trace ([`trace`]).
//!
//! Persistence and Markdown conversion are ports ([`ports`]); [`memory`]
//! holds in-memory adapters for tests and the zero-config profile.

mod config;
mod hub;
pub mod markdown;
pub mod memory;
mod metrics;
mod persist;
pub mod placement;
pub mod ports;
pub mod protocol;
mod room;
pub mod sql;
pub mod trace;
pub mod tree;
pub mod ws;

pub use config::CollabConfig;
pub use hub::{Access, CloseReason, Connection, Hub, JoinRequest, Outbound, PeerSender, RoomKey};
pub use markdown::FormatCodec;

pub use ports::{
    DocumentFiles, MarkdownCodec, MarkdownVersion, Seq, StoredUpdate, UpdateStore, VersionReason,
};
pub use tree::{DocTree, Element, Node, TextRun};
