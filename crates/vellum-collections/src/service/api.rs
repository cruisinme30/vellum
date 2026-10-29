// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Inputs and outputs of the collections operations. Every type is
//! serde + JSON Schema, so the gateway publishes it in OpenAPI and
//! GraphQL and the web app generates its client from it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{CollectionId, DocId};

use crate::compute::ComputedRow;
use crate::error::Diagnostic;
use crate::schema::{Aggregation, NumberFormat, Property, PropertyType, Schema, SelectOption};
use crate::value::Value;
use crate::view::{ViewDef, ViewKind};
use crate::workspace::Changeset;

/// `collections.list` takes no input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCollections {}

/// Names one collection.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionKey {
    /// The collection's ID (the `id` in `schema.yaml`).
    pub collection_id: CollectionId,
}

/// Names one row.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RowKey {
    /// The collection.
    pub collection_id: CollectionId,
    /// The row (the `id` in its front matter).
    pub row_id: DocId,
}

/// Runs a saved view, with optional ad-hoc changes on top.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryInput {
    /// The collection.
    pub collection_id: CollectionId,
    /// A saved view's name; the first view when absent.
    #[serde(default)]
    pub view: Option<String>,
    /// Another layout for this query only.
    #[serde(default)]
    pub kind: Option<ViewKind>,
    /// Extra filters, added to the view's (`Status = Done`).
    #[serde(default)]
    pub filter: Vec<String>,
    /// Sort keys replacing the view's (`Due desc`).
    #[serde(default)]
    pub sort: Vec<String>,
    /// Group-by property, replacing the view's.
    #[serde(default)]
    pub group: Option<String>,
    /// Hidden properties, replacing the view's.
    #[serde(default)]
    pub hide: Vec<String>,
    /// Most rows to return.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Exports a view as CSV.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportCsvInput {
    /// The collection.
    pub collection_id: CollectionId,
    /// A saved view's name; the first view when absent. The export has
    /// that view's columns, filters and order, and every matching row.
    #[serde(default)]
    pub view: Option<String>,
}

/// A CSV export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CsvExport {
    /// A suggested file name (`Requirements - Open.csv`).
    pub file_name: String,
    /// The CSV text: a header row, then one line per row.
    pub csv: String,
}

/// Adds the rows of a CSV file to an existing collection.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportCsvInput {
    /// The collection.
    pub collection_id: CollectionId,
    /// The CSV text. Headers match property names or keys; relation cells
    /// match related rows by title.
    pub csv: String,
    /// Check the file and report what would happen, without writing.
    #[serde(default)]
    pub dry_run: bool,
}

/// Creates a new collection from a CSV file, inferring property types.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateFromCsvInput {
    /// The drive folder to create the `Title.collection` folder in.
    pub folder: String,
    /// The collection's title.
    pub title: String,
    /// The CSV text; the first row is the header.
    pub csv: String,
    /// Infer the schema and report it, without writing.
    #[serde(default)]
    pub dry_run: bool,
}

/// What a CSV import did, or would do on a dry run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CsvImported {
    /// Whether anything was written.
    pub written: bool,
    /// Rows created (or that would be).
    pub rows: usize,
    /// Headers that matched no writable property and were ignored.
    pub skipped_columns: Vec<String>,
    /// Problems with individual cells; their values were kept as text or
    /// left empty.
    pub diagnostics: Vec<Diagnostic>,
    /// The collection after the import: its properties (inferred, for a
    /// new collection) and views.
    pub collection: CollectionInfo,
    /// Row files written, including links mirrored into other collections.
    pub changed: Changeset,
}

/// Creates a row.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRowInput {
    /// The collection.
    pub collection_id: CollectionId,
    /// Values by property key or name.
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    /// The row page's Markdown body.
    #[serde(default)]
    pub body: String,
}

/// Changes some values of a row, and optionally its body.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRowInput {
    /// The collection.
    pub collection_id: CollectionId,
    /// The row.
    pub row_id: DocId,
    /// Values by property key or name; an `empty` value clears one.
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
    /// A new body, when it changes.
    #[serde(default)]
    pub body: Option<String>,
}

/// One collection in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    /// The collection's ID.
    pub id: CollectionId,
    /// Its title.
    pub title: String,
    /// The `name.collection` folder.
    pub path: String,
    /// How many rows it has.
    pub rows: usize,
    /// Saved view names, in tab order.
    pub views: Vec<String>,
}

/// The collections the caller can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CollectionList {
    /// Ordered by title.
    pub collections: Vec<CollectionSummary>,
}

/// A relation's target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationInfo {
    /// The related collection.
    pub to: CollectionId,
    /// The paired property there, for a two-way relation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub back: Option<String>,
}

/// How a rollup is computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RollupInfo {
    /// The relation property followed.
    pub via: String,
    /// The property read on related rows.
    pub prop: String,
    /// How values are combined.
    pub agg: Aggregation,
}

/// One property, as the UI needs it to render headers and editors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertyInfo {
    /// Stable key used in row front matter.
    pub key: String,
    /// Display name.
    pub name: String,
    /// The type name (`title`, `select`, `formula`, …).
    #[serde(rename = "type")]
    pub kind: String,
    /// Whether the value is computed (and read-only).
    pub computed: bool,
    /// Options of a select or multi-select.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// For a relation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<RelationInfo>,
    /// For a rollup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollup: Option<RollupInfo>,
    /// For a formula: its source text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// Number display format, where it applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<NumberFormat>,
}

impl From<&Property> for PropertyInfo {
    fn from(p: &Property) -> Self {
        let mut info = Self {
            key: p.key.clone(),
            name: p.name.clone(),
            kind: p.kind.name().to_owned(),
            computed: !p.kind.is_stored(),
            options: p.kind.options().to_vec(),
            relation: None,
            rollup: None,
            formula: None,
            format: None,
        };
        match &p.kind {
            PropertyType::Number { format } => info.format = Some(*format),
            PropertyType::Relation { to, back } => {
                info.relation = Some(RelationInfo {
                    to: *to,
                    back: back.clone(),
                });
            }
            PropertyType::Rollup {
                via,
                prop,
                agg,
                format,
            } => {
                info.rollup = Some(RollupInfo {
                    via: via.clone(),
                    prop: prop.clone(),
                    agg: *agg,
                });
                info.format = Some(*format);
            }
            PropertyType::Formula { formula, format } => {
                info.formula = Some(formula.source().to_owned());
                info.format = Some(*format);
            }
            _ => {}
        }
        info
    }
}

/// A saved view, in the text form `schema.yaml` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewInfo {
    /// The view's name.
    pub name: String,
    /// The layout.
    pub kind: ViewKind,
    /// Filters, all of which must match.
    pub filters: Vec<String>,
    /// Sort keys, most significant first.
    pub sorts: Vec<String>,
    /// Group-by property key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Hidden property keys.
    pub hidden: Vec<String>,
    /// Date property for calendar and timeline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// End-date property for timeline bars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// Most rows shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl From<&ViewDef> for ViewInfo {
    fn from(v: &ViewDef) -> Self {
        Self {
            name: v.name.clone(),
            kind: v.kind,
            filters: v.filters.iter().map(crate::view::Filter::to_text).collect(),
            sorts: v.sorts.iter().map(crate::view::Sort::to_text).collect(),
            group: v.group.clone(),
            hidden: v.hidden.clone(),
            date: v.date.clone(),
            end: v.end.clone(),
            limit: v.limit,
        }
    }
}

/// A collection's schema and saved views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionInfo {
    /// The collection's ID.
    pub id: CollectionId,
    /// Its title.
    pub title: String,
    /// The `name.collection` folder.
    pub path: String,
    /// Properties in column order.
    pub properties: Vec<PropertyInfo>,
    /// Saved views in tab order.
    pub views: Vec<ViewInfo>,
    /// Problems found while loading its files.
    pub diagnostics: Vec<Diagnostic>,
}

impl CollectionInfo {
    pub(crate) fn new(schema: &Schema, path: String, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            id: schema.id,
            title: schema.title.clone(),
            path,
            properties: schema.properties.iter().map(PropertyInfo::from).collect(),
            views: schema.views.iter().map(ViewInfo::from).collect(),
            diagnostics,
        }
    }
}

/// One row with every value, stored and computed.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RowDetail {
    /// The collection.
    pub collection_id: CollectionId,
    /// The row page's path.
    pub path: String,
    /// The row's title.
    pub title: String,
    /// Values by property key; computed cells may hold an inline error.
    pub row: ComputedRow,
    /// The page's Markdown body.
    pub body: String,
}

/// The result of a row write.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RowWritten {
    /// The row after the write; absent after a delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<RowDetail>,
    /// Every row file written or removed, including mirrored relations.
    pub changed: Changeset,
}
