// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The books service: the engine behind the gateway for `name.book/`
//! folders (ADR 0013, D143 to D145).
//!
//! [`BooksService`] implements [`vellum_contracts::Service`] over a
//! [`vellum_store::BlobStore`] holding the book folders and an [`Access`]
//! port. Every request loads the book from the store, so edits made on
//! disk are seen at once; every edit saves only what changed, guarded by
//! etags, then mirrors the book's section restrictions into the
//! authorization engine.
//!
//! | Operation | Route | Scope |
//! |---|---|---|
//! | `books.list` | `GET /v1/books` | `read` |
//! | `books.get` | `GET /v1/books/{book_id}` | `read` |
//! | `books.get_section` | `GET /v1/books/{book_id}/sections/{section_id}` | `read` |
//! | `books.add_section` | `POST /v1/books/{book_id}/sections` | `write:direct` |
//! | `books.move_section` | `POST /v1/books/{book_id}/sections/{section_id}/move` | `write:direct` |
//! | `books.update_section` | `PATCH /v1/books/{book_id}/sections/{section_id}` | `write:direct` |
//! | `books.remove_section` | `DELETE /v1/books/{book_id}/sections/{section_id}` | `write:direct` |
//! | `books.set_section_access` | `PUT /v1/books/{book_id}/sections/{section_id}/access` | `write:direct` |
//! | `books.list_snapshots` | `GET /v1/books/{book_id}/snapshots` | `read` |
//! | `books.create_snapshot` | `POST /v1/books/{book_id}/snapshots` | `write:direct` |
//! | `books.diff` | `GET /v1/books/{book_id}/diff` | `read` |
//!
//! Access (D144): opening a book needs read access to its `.book` node.
//! Editing needs edit access, and changing section access needs manage
//! access. A restricted section is visible to book managers and owners,
//! and to others only when they hold one of its effective `restrict_to`
//! principals; anything else is "not found". Removing a section needs
//! sight of everything under it.

pub mod access;
pub mod api;
pub mod files;
pub mod mirror;
mod service;

pub use access::{Access, AuthzAccess, MirrorReport, SpaceOf};
pub use mirror::Restrictions;
pub use service::BooksService;
