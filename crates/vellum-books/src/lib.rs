// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Books: a document that is a tree of documents (brief §7, R-BK-1 to
//! R-BK-10, ADR 0013).
//!
//! A book is a `name.book/` folder:
//!
//! ```text
//! handbook.book/
//!   book.yaml                  # Vellum settings: id, numbering, depth, per-section access
//!   SUMMARY.md                 # order and nesting, mdBook syntax
//!   book.toml                  # generated, so unmodified `mdbook build` works
//!   README.md                  # the introduction (a prefix chapter)
//!   onboarding/README.md       # a section with children
//!   onboarding/first-week.md   # a leaf section
//!   .snapshots/v1.0.json       # a tagged snapshot
//! ```
//!
//! | Module | What it does |
//! |---|---|
//! | [`manifest`] | `book.yaml` |
//! | [`summary`] | `SUMMARY.md` grammar and canonical form |
//! | [`book`] | the book in memory; loading repairs outside edits |
//! | [`numbering`], [`toc`] | computed numbers, the table of contents, the export tree |
//! | [`ops`] | add, reorder, nest, rename, remove; files follow the tree |
//! | [`xref`] | cross-references rendered as `§2.3 Title` |
//! | [`subtree`] | move and copy subtrees within and between books |
//! | [`convert`] | headings ⇄ sections |
//! | [`access`] | per-section restrictions (narrowing only), owners, reviewers |
//! | [`snapshot`] | tags and diffs |
//! | [`publish`] | static site and mdBook tree |
//! | [`fs`] | the local-folder adapter |
//!
//! The crate is pure domain logic plus a local-disk adapter: no database,
//! no network. Identity is always the section id from the file's front
//! matter, never its path.

pub mod access;
pub mod book;
pub mod convert;
pub mod error;
pub mod fs;
pub mod id;
pub mod manifest;
pub mod markdown;
pub mod numbering;
pub mod ops;
pub mod path;
pub mod publish;
pub mod snapshot;
pub mod subtree;
pub mod summary;
pub mod toc;
pub mod xref;

pub use book::{Book, Repair};
pub use error::{BookError, Result};
pub use id::{BookId, IdSource, NodeId, SequentialIds};
