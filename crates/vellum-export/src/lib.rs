// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Export to PDF, Word and Google Docs; import from Word, Google Docs,
//! Notion, Confluence, Outline and Markdown folders.
//!
//! # Export
//!
//! Every export starts from [`vellum_format::Document`]s. [`lower()`] turns
//! them into a [`Flow`]: the document as print and office formats see it, with
//! containers and data fences already mapped to the shapes the PRD block
//! catalogue asks for, live blocks frozen to their current state, charts and
//! diagrams drawn as SVG and images loaded. A book export first goes through
//! [`book::lower_book`], which picks a section, subtree or the whole book,
//! turns depth into heading levels, numbers the sections and resolves
//! cross-references.
//!
//! A [`Flow`] then goes to one backend:
//!
//! | Format | Backend | Notes |
//! |---|---|---|
//! | Print PDF | [`pdf::PdfEngine`] (Typst) | vector charts, real maths, PDF outline, page-number cross-references |
//! | Screen PDF | [`screen::Gotenberg`] (optional) | Chromium renders the page's own HTML |
//! | Word | [`docx::write`] | heading styles, real lists and tables, SVG with PNG fallback, OMML equations, `PAGEREF` cross-references |
//! | Google Docs | [`gdocs::Drive`] | the Word file, uploaded with conversion through the Drive API |
//! | HTML | [`html::write`] | one self-contained file, no script, images embedded |
//!
//! Markdown export ([`bundle::markdown`]) writes the page's own Vellum
//! Markdown with live blocks frozen, and [`bundle::zip`] packs several
//! files (pages and their attachments, or several exports) into one zip.
//!
//! # Import
//!
//! [`import::import`] reads a Markdown folder, Obsidian vault, Outline,
//! Docmost, Notion or Confluence export, or a Word file (and
//! [`import::google_doc`] a Google Doc) into an [`import::ImportSet`]:
//! canonical Vellum Markdown pages with fresh IDs and links rewritten to
//! `vellum://doc/<id>`, their attachments, and databases as collections.
//! Callers write the files through the store.
//!
//! Every export and import runs in a `tracing` span and records metrics
//! through the global OpenTelemetry meter ([`metrics`]).

pub mod book;
pub mod bundle;
mod calc;
pub mod docx;
pub mod error;
pub mod flow;
pub mod gdocs;
pub mod html;
pub mod import;
pub mod lower;
pub mod math;
pub mod metrics;
pub mod pdf;
pub mod ports;
pub mod screen;

pub use error::{ExportError, ExportWarning, ImportError};
pub use flow::Flow;
pub use lower::{LowerOptions, Target, lower};
pub use ports::Ports;
