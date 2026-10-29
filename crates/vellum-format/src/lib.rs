// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Vellum Markdown dialect (v1): parser, AST and canonical serializer.
//!
//! Vellum Markdown is `CommonMark` 0.31 + GFM, YAML front matter, `:::`
//! containers with Pandoc attributes, data fences, `$$` math and block IDs.
//! Raw HTML is never interpreted. The [`ast`] module is the document model.
//!
//! [`parse`] never fails: problems are reported as [`Diagnostic`]s next to a
//! readable fallback, so a page always renders.

pub mod ast;
pub mod attrs;
mod convert;
pub mod diagnostic;
mod extras;
pub mod front_matter;
mod prepass;

pub use ast::{
    Alignment, Attributes, Block, Container, ContainerKind, Document, Fence, FenceKind, Inline,
    List, ListItem, Table,
};
pub use convert::plain_text;
pub use diagnostic::{Diagnostic, DiagnosticCode};
pub use front_matter::{FrontMatter, Status};

/// A parsed document and the problems found while reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The document, in canonical form.
    pub document: Document,
    /// Problems found, in source order within each kind.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a Vellum Markdown file.
///
/// Line endings are normalised to `\n` and a leading byte order mark is
/// dropped. Parsing never fails.
#[must_use]
#[tracing::instrument(level = "debug", skip_all, fields(bytes = src.len()))]
pub fn parse(src: &str) -> Parsed {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let src = if src.contains('\r') {
        std::borrow::Cow::Owned(src.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(src)
    };
    let mut diagnostics = Vec::new();
    let (front_matter, body, first_line) = match front_matter::split(&src) {
        Some((yaml, body, lines)) => (
            Some(front_matter::parse(yaml, &mut diagnostics)),
            body,
            lines + 1,
        ),
        None => (None, src.as_ref(), 1),
    };
    let blocks = prepass::blocks(body, first_line, &mut diagnostics);
    Parsed {
        document: Document {
            front_matter,
            blocks,
        },
        diagnostics,
    }
}
