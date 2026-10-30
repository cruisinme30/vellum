// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The error type shared by every book operation.

use crate::id::NodeId;

/// Why a book operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BookError {
    /// A string is not a valid ULID.
    #[error("invalid id {value:?}: {reason}")]
    InvalidId {
        /// The rejected value.
        value: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A chapter path is unsafe or malformed.
    #[error("invalid chapter path {path:?}: {reason}")]
    InvalidPath {
        /// The rejected path.
        path: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A principal is not of the form `user:<name>` or `group:<name>`.
    #[error("invalid principal {0:?}: expected user:<name> or group:<name>")]
    InvalidPrincipal(String),
    /// `book.yaml` could not be read.
    #[error("book.yaml: {0}")]
    Manifest(String),
    /// `SUMMARY.md` does not follow the mdBook grammar.
    #[error("SUMMARY.md line {line}: {message}")]
    Summary {
        /// The 1-based line where parsing failed.
        line: usize,
        /// What went wrong.
        message: String,
    },
    /// The manifest, the summary and the files disagree in a way that
    /// cannot be repaired automatically.
    #[error("inconsistent book: {0}")]
    Inconsistent(String),
    /// No node has this id.
    #[error("no section with id {0}")]
    UnknownNode(NodeId),
    /// A position does not point at an item of the book.
    #[error("no item at {0}")]
    UnknownPosition(String),
    /// The operation would nest sections deeper than `max_depth`.
    #[error("the book allows {max} levels; this would need {needed}")]
    TooDeep {
        /// The book's `max_depth`.
        max: u8,
        /// The depth the operation would create.
        needed: usize,
    },
    /// The operation is not allowed by the mdBook structure, for example
    /// nesting under a prefix chapter or moving a section into itself.
    #[error("not allowed: {0}")]
    NotAllowed(String),
    /// A section restriction would widen access instead of narrowing it.
    #[error(
        "section {node} cannot grant {principal}: an enclosing section does not"
    )]
    WidensAccess {
        /// The section being restricted.
        node: NodeId,
        /// The principal the enclosing section excludes.
        principal: String,
    },
    /// A snapshot tag is malformed or already exists.
    #[error("snapshot tag {tag:?}: {reason}")]
    Tag {
        /// The tag.
        tag: String,
        /// What is wrong.
        reason: &'static str,
    },
    /// A snapshot file could not be read or written.
    #[error("snapshot: {0}")]
    Snapshot(String),
    /// The page renderer failed.
    #[error("render failed for {path}: {message}")]
    Render {
        /// The chapter being rendered.
        path: String,
        /// The renderer's message.
        message: String,
    },
    /// A file-system operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory involved.
        path: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Shorthand for results of book operations.
pub type Result<T, E = BookError> = std::result::Result<T, E>;
