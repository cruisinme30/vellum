// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What search needs from collections: each row as plain text with its
//! computed values rendered, and a feed of the rows to index again after
//! a write.
//!
//! Computed values (formulas, rollups, created time) are never written to
//! row files, so search cannot learn them by reading the files. The
//! service renders them here instead, and names every row whose values
//! may have changed, including rows whose rollups read the edited row.
//! Search sits in the same layer and cannot depend on this crate, so the
//! application wires the two together with a small adapter.

use std::collections::BTreeSet;

use vellum_kernel::{CollectionId, DocId};

use crate::compute::Computer;
use crate::schema::PropertyType;
use crate::workspace::Changeset;

use super::state::State;

/// Receives the rows to index again after a write.
///
/// Called after the files are written, while the service still holds its
/// write lock, so implementations must return quickly: queue the IDs and
/// index them elsewhere. A row that no longer exists reads as `None` from
/// [`CollectionsService::index_row`](super::CollectionsService::index_row)
/// and should be removed from the index.
pub trait IndexFeed: Send + Sync {
    /// Rows whose stored or computed values may have changed.
    fn rows_changed(&self, rows: Vec<DocId>);
}

/// One property of a row, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexField {
    /// Stable property key.
    pub key: String,
    /// Display name, for `Name: value` text.
    pub name: String,
    /// The value as shown in the app: numbers formatted, relations as
    /// related titles, dates as `YYYY-MM-DD`.
    pub text: String,
    /// Whether the value is computed (formula, rollup, created time).
    pub computed: bool,
}

/// A row as search should see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowDocument {
    /// The collection; its authorization node is `Id(collection)`.
    pub collection: CollectionId,
    /// The collection's title.
    pub collection_title: String,
    /// The `name.collection` folder.
    pub folder: String,
    /// The row.
    pub row: DocId,
    /// The row's title.
    pub title: String,
    /// The row page's path.
    pub path: String,
    /// Every non-empty property except the title, in column order.
    /// Cells whose formula fails are left out.
    pub fields: Vec<IndexField>,
    /// The page's Markdown body.
    pub body: String,
}

/// The rows a write touched (deleted ones included, so search drops
/// them), plus every row whose rollups read them.
pub(super) fn affected_rows(state: &State, changed: &Changeset) -> Vec<DocId> {
    let mut rows: BTreeSet<DocId> = state
        .ws
        .affected(changed)
        .into_iter()
        .map(|r| r.row)
        .collect();
    rows.extend(changed.deleted.iter().map(|r| r.row));
    rows.into_iter().collect()
}

/// Every loaded row.
pub(super) fn all_rows(state: &State) -> Vec<DocId> {
    state
        .folders
        .values()
        .flat_map(|f| f.rows.keys().copied())
        .collect()
}

/// Renders one row, or `None` when no loaded collection has it.
pub(super) fn row_document(
    state: &State,
    computer: &mut Computer<'_>,
    row: DocId,
) -> Option<RowDocument> {
    let (id, folder) = state
        .folders
        .iter()
        .find(|(_, f)| f.rows.contains_key(&row))?;
    let (path, _) = folder.rows.get(&row)?;
    let collection = state.ws.collection(*id)?;
    let schema = collection.schema();
    let stored = collection.row(row)?;
    let values = computer.row(*id, row)?;
    let fields = schema
        .properties
        .iter()
        .filter(|p| !matches!(p.kind, PropertyType::Title))
        .filter_map(|p| {
            let cell = values.cells.get(&p.key)?;
            if cell.as_ref().is_ok_and(crate::value::Value::is_empty) || cell.is_err() {
                return None;
            }
            Some(IndexField {
                key: p.key.clone(),
                name: p.name.clone(),
                text: crate::view::display(computer, p, cell),
                computed: !p.kind.is_stored(),
            })
        })
        .collect();
    Some(RowDocument {
        collection: *id,
        collection_title: schema.title.clone(),
        folder: folder.path.as_str().to_owned(),
        row,
        title: stored.title(schema),
        path: path.as_str().to_owned(),
        fields,
        body: stored.body.clone(),
    })
}
