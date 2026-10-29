// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The ports a room needs: an update log, the document's Markdown file and
//! the dialect codec.
//!
//! Adapters live with the systems they wrap: the Postgres and `SQLite` log
//! in `vellum-db`, files in `vellum-store`, the codec over `vellum-format`.
//! [`crate::memory`] provides in-memory adapters.

use async_trait::async_trait;
use vellum_kernel::{Result, UserId};

use crate::hub::RoomKey;
use crate::tree::DocTree;

/// Position of a record in a room's update log; increases with each append.
pub type Seq = u64;

/// One record of a room's update log: a Yjs v1 update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUpdate {
    /// Log position.
    pub seq: Seq,
    /// Yjs update, lib0 v1 encoding.
    pub update: Vec<u8>,
}

/// The append-only log of CRDT updates, one log per room.
///
/// Adapters seal each record with the document's data key before it
/// leaves the process (envelope encryption at rest is on by default).
#[async_trait]
pub trait UpdateStore: Send + Sync + 'static {
    /// Every record of the room, in `seq` order.
    async fn load(&self, room: RoomKey) -> Result<Vec<StoredUpdate>>;

    /// Appends one record and returns its position.
    async fn append(&self, room: RoomKey, update: Vec<u8>) -> Result<Seq>;

    /// Writes the first record only if the log is empty, and says whether
    /// it did. Rooms seed a log from the Markdown file through this, so two
    /// nodes opening the same document never both import it.
    async fn seed(&self, room: RoomKey, update: Vec<u8>) -> Result<bool>;

    /// Atomically replaces every record up to and including `through` with
    /// one record at `through` holding `merged`. Later records are kept.
    async fn compact(&self, room: RoomKey, through: Seq, merged: Vec<u8>) -> Result<()>;
}

/// Why a Markdown version was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionReason {
    /// Nobody edited for [`crate::CollabConfig::idle_version_after`].
    Idle,
    /// An editor pressed Done.
    Done,
    /// The last editor left and the room closed.
    Closed,
    /// The process is draining.
    Shutdown,
}

impl VersionReason {
    /// A stable label for logs and metrics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Closed => "closed",
            Self::Shutdown => "shutdown",
        }
    }
}

/// A Markdown version of a document, ready to be written to its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownVersion {
    /// The document in canonical Vellum Markdown.
    pub markdown: String,
    /// What triggered the write.
    pub reason: VersionReason,
    /// Users who edited since the previous version, for attribution.
    pub editors: Vec<UserId>,
}

/// The document's Markdown file: where a room starts and where its
/// versions go.
#[async_trait]
pub trait DocumentFiles: Send + Sync + 'static {
    /// The current Markdown, or `None` for a document with no file yet.
    async fn read_markdown(&self, room: RoomKey) -> Result<Option<String>>;

    /// Writes a new version of the file (and its version history entry).
    async fn write_version(&self, room: RoomKey, version: MarkdownVersion) -> Result<()>;
}

/// Converts between Vellum Markdown and the editor's node tree.
///
/// The production adapter parses with `vellum-format` and serializes with
/// its canonical serializer, so `to_markdown(parse_markdown(md))` is the
/// canonical form of `md`.
pub trait MarkdownCodec: Send + Sync + 'static {
    /// Serializes a tree as canonical Vellum Markdown.
    ///
    /// # Errors
    ///
    /// When the tree holds a node the dialect cannot express.
    fn to_markdown(&self, doc: &DocTree) -> Result<String>;

    /// Parses Vellum Markdown into a tree. Parsing never fails on content
    /// (unknown blocks become readable fallbacks).
    ///
    /// # Errors
    ///
    /// Only on an internal fault.
    fn parse_markdown(&self, markdown: &str) -> Result<DocTree>;
}
