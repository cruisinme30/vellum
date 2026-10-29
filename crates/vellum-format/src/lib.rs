// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Vellum Markdown dialect (v1): parser, AST and canonical serializer.
//!
//! Vellum Markdown is `CommonMark` 0.31 + GFM, YAML front matter, `:::`
//! containers with Pandoc attributes, data fences, `$$` math and block IDs.
//! Raw HTML is never interpreted. The [`ast`] module is the document model.
//! Parsing never fails: problems are
//! reported as [`Diagnostic`]s next to a readable fallback.

pub mod ast;
pub mod diagnostic;
pub mod front_matter;

pub use ast::{
    Alignment, Attributes, Block, Container, ContainerKind, Document, Fence, FenceKind, Inline,
    List, ListItem, Table,
};
pub use diagnostic::{Diagnostic, DiagnosticCode};
pub use front_matter::{FrontMatter, Status};
