// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The collections engine: schemas, rows, relations, rollups and formulas.
//!
//! A collection is a folder: `schema.yaml` plus one page per row in
//! `rows/*.md`. This crate turns those files into typed data and back, and
//! does everything that needs no I/O:
//!
//! | Module | What it does |
//! |---|---|
//! | [`schema`] | `schema.yaml`: 13 property types, select options, saved views |
//! | [`row`] | row pages: front matter ⇄ typed values, slugs |
//! | [`workspace`] | loaded collections, validated edits, two-way relations |
//! | [`compute`] | formulas, rollups and created times, memoised |
//! | [`formula`] | the safe expression language: parser, evaluator, positioned errors |
//! | [`view`] | table, board, gallery, calendar, timeline and list views |
//! | [`mod@csv`] | CSV import (with type inference) and export |
//! | [`fence`] | the `collection` fence and `source=collection:` charts |
//!
//! The save path is: apply an [`Edit`] to the [`Workspace`], write the
//! files named in the [`Changeset`], then recompute
//! [`Workspace::affected`] rows with a [`Computer`] and push them to the
//! index and search. Computed values are never written to files.
//!
//! ```
//! use vellum_collections::{Collection, Computer, DateValue, Workspace, view};
//!
//! let schema = "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\ntitle: Tasks\nproperties:\n  - {key: t, name: Task, type: title}\n  - {key: est, name: Estimate, type: number}\n  - {key: dbl, name: Double, type: formula, f: 'Estimate * 2'}\n";
//! let row = "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C3G\nt: Write docs\nest: 3\n---\n";
//! let (collection, diagnostics) = Collection::load(schema, [("rows/write-docs.md", row)])?;
//! assert!(diagnostics.is_empty());
//! let id = collection.id();
//! let mut ws = Workspace::new();
//! ws.insert(collection)?;
//! let now = DateValue::ymd(2026, 10, 29).unwrap_or_else(|| unreachable!());
//! let mut computer = Computer::new(&ws, now);
//! let all = ws.collection(id).and_then(|c| c.schema().view(None)).cloned().unwrap_or_default();
//! let result = view::query(&mut computer, id, &all)?;
//! assert_eq!(result.groups[0].rows[0].cells[2].display, "6");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod compute;
pub mod csv;
pub mod error;
pub mod fence;
pub mod formula;
pub mod row;
pub mod schema;
pub mod telemetry;
pub mod value;
pub mod view;
pub mod workspace;
pub mod yaml;

pub use compute::{Cell, CellError, ComputedRow, Computer};
pub use error::{Diagnostic, EditError, SchemaError, Severity};
pub use fence::{ChartData, ChartSource, CollectionEmbed, EmbedError};
pub use formula::{Formula, FormulaError, Span};
pub use row::Row;
pub use schema::{Aggregation, NumberFormat, Property, PropertyType, Schema, SelectOption};
pub use value::{DateValue, Value};
pub use view::{Filter, FilterOp, QueryError, Sort, ViewDef, ViewKind, ViewResult};
pub use workspace::{Changeset, Collection, Edit, RowRef, Workspace};
