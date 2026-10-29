// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Export to PDF, Word and Google Docs; import from Word, Google Docs,
//! Notion, Confluence, Outline and Markdown folders.
//!
//! # Export
//!
//! Every export starts from [`vellum_format::Document`]s. [`lower`] turns
//! them into a [`Flow`]: the document as print and office formats see it,
//! with containers and data fences already mapped to the shapes the PRD
//! block catalogue asks for, live blocks frozen to their current state,
//! charts and diagrams drawn as SVG and images loaded. A book export first
//! goes through [`book`], which picks a section, subtree or the whole book,
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
//!
//! # Import
//!
//! [`import`] reads an outside source into an [`import::ImportSet`]: Vellum
//! documents with fresh IDs, links rewritten to `vellum://doc/<id>`, and
//! their attachments. Callers serialise each document with
//! `vellum_format::serialize` and write it through the store.
//!
//! Every export and import runs in a `tracing` span and records metrics
//! through the global OpenTelemetry meter ([`metrics`]).

pub mod book;
pub mod data;
pub mod docx;
pub mod error;
pub mod flow;
pub mod gdocs;
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
