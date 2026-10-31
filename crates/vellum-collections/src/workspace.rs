// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Collections in memory, edits, and two-way relations.
//!
//! A [`Workspace`] holds every loaded collection so that relations and
//! rollups can cross between them. Edits go through [`Workspace::apply`],
//! which validates values against the schema, keeps both sides of every
//! two-way relation in step, and returns a [`Changeset`] naming the row
//! files to write and the rows whose computed values must be refreshed.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use vellum_kernel::{CollectionId, DocId};

use crate::error::{Diagnostic, EditError, SchemaError};
use crate::row::{Row, relation_value, slug};
use crate::schema::{Property, PropertyType, Schema};
use crate::value::Value;

/// File name of the schema inside a collection folder.
pub const SCHEMA_FILE: &str = "schema.yaml";

/// Folder holding row pages inside a collection folder.
pub const ROWS_DIR: &str = "rows";

/// A collection: its schema and rows, keyed by row ID (creation order).
#[derive(Debug, Clone)]
pub struct Collection {
    schema: Schema,
    rows: BTreeMap<DocId, Row>,
}

impl Collection {
    /// An empty collection.
    #[must_use]
    pub const fn new(schema: Schema) -> Self {
        Self {
            schema,
            rows: BTreeMap::new(),
        }
    }

    /// Loads a collection folder from its `schema.yaml` text and its row
    /// pages as `(path, text)` pairs.
    ///
    /// Rows never fail to load; see [`Row::parse`]. A row whose ID repeats
    /// an earlier one gets a fresh ID and a warning.
    ///
    /// # Errors
    ///
    /// Returns a [`SchemaError`] when `schema.yaml` is invalid.
    pub fn load<P, T>(
        schema_yaml: &str,
        rows: impl IntoIterator<Item = (P, T)>,
    ) -> Result<(Self, Vec<Diagnostic>), SchemaError>
    where
        P: AsRef<str>,
        T: AsRef<str>,
    {
        let schema = Schema::from_yaml(schema_yaml)?;
        let mut collection = Self::new(schema);
        let mut diags = Vec::new();
        for (path, text) in rows {
            let (mut row, mut d) =
                Row::parse(text.as_ref(), &collection.schema, path.as_ref());
            diags.append(&mut d);
            if collection.rows.contains_key(&row.id) {
                let fresh = DocId::generate();
                diags.push(Diagnostic::warning(
                    path.as_ref(),
                    format!(
                        "row id {} is used twice; assigned {fresh}",
                        row.id
                    ),
                ));
                row.id = fresh;
            }
            let _ = collection.rows.insert(row.id, row);
        }
        Ok((collection, diags))
    }

    /// The collection's ID.
    #[must_use]
    pub const fn id(&self) -> CollectionId {
        self.schema.id
    }

    /// The schema.
    #[must_use]
    pub const fn schema(&self) -> &Schema {
        &self.schema
    }

    /// One row.
    #[must_use]
    pub fn row(&self, id: DocId) -> Option<&Row> {
        self.rows.get(&id)
    }

    /// All rows in creation order.
    #[must_use = "iterators are lazy"]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Row> {
        self.rows.values()
    }

    /// Number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row file names (`rows/<slug>.md`), unique within the folder: rows
    /// sharing a slug get `-2`, `-3`… in creation order.
    #[must_use]
    pub fn file_names(&self) -> BTreeMap<DocId, String> {
        let mut used: BTreeMap<String, usize> = BTreeMap::new();
        self.rows
            .values()
            .map(|row| {
                let base = slug(&row.title(&self.schema));
                let n = used.entry(base.clone()).or_insert(0);
                *n += 1;
                let name = if *n == 1 {
                    format!("{ROWS_DIR}/{base}.md")
                } else {
                    format!("{ROWS_DIR}/{base}-{n}.md")
                };
                (row.id, name)
            })
            .collect()
    }

    /// Every file of the folder as `(path, text)`: `schema.yaml` first,
    /// then the rows.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::Yaml`] if the schema cannot be serialised.
    pub fn files(&self) -> Result<Vec<(String, String)>, SchemaError> {
        let mut out = vec![(SCHEMA_FILE.to_owned(), self.schema.to_yaml()?)];
        let names = self.file_names();
        for row in self.rows.values() {
            let name = names.get(&row.id).cloned().unwrap_or_default();
            out.push((name, row.to_markdown(&self.schema)));
        }
        Ok(out)
    }

    /// Adds a row as-is (no relation syncing); for building new collections.
    pub(crate) fn insert_row(&mut self, row: Row) {
        let _ = self.rows.insert(row.id, row);
    }

    fn row_mut(&mut self, id: DocId) -> Result<&mut Row, EditError> {
        self.rows.get_mut(&id).ok_or(EditError::UnknownRow(id))
    }
}

/// A row in a specific collection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct RowRef {
    /// The collection.
    pub collection: CollectionId,
    /// The row.
    pub row: DocId,
}

/// A change to apply to a workspace.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Edit {
    /// Creates a row. Values are keyed by property key or name.
    CreateRow {
        /// The collection.
        collection: CollectionId,
        /// The new row's ID; generated when `None`.
        id: Option<DocId>,
        /// Initial values.
        values: BTreeMap<String, Value>,
        /// Initial page body.
        body: String,
    },
    /// Sets some values (and optionally the body) of a row; `Value::Empty`
    /// clears a value.
    UpdateRow {
        /// The collection.
        collection: CollectionId,
        /// The row.
        row: DocId,
        /// Values by property key or name.
        values: BTreeMap<String, Value>,
        /// A new body, if it changes.
        body: Option<String>,
    },
    /// Replaces a row wholesale, as when a file changed on disk; relation
    /// differences are mirrored to the other side.
    ReplaceRow {
        /// The collection.
        collection: CollectionId,
        /// The row as read from its file.
        row: Row,
    },
    /// Deletes a row and removes every link to it.
    DeleteRow {
        /// The collection.
        collection: CollectionId,
        /// The row.
        row: DocId,
    },
}

/// What an edit touched.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Changeset {
    /// Rows whose files must be written (created or changed).
    pub written: BTreeSet<RowRef>,
    /// Rows whose files must be deleted.
    pub deleted: BTreeSet<RowRef>,
}

impl Changeset {
    fn merge(&mut self, other: Self) {
        self.written.extend(other.written);
        self.deleted.extend(other.deleted);
        self.written.retain(|r| !self.deleted.contains(r));
    }
}

/// Every loaded collection.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    collections: BTreeMap<CollectionId, Collection>,
}

impl Workspace {
    /// An empty workspace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a collection, then checks relations and rollups
    /// against the collections they point to (where those are loaded).
    ///
    /// # Errors
    ///
    /// Returns a [`SchemaError`] and leaves the workspace unchanged when a
    /// relation does not pair up or a rollup reads a missing property.
    pub fn insert(
        &mut self,
        collection: Collection,
    ) -> Result<(), SchemaError> {
        let id = collection.id();
        let previous = self.collections.insert(id, collection);
        if let Err(e) = self.check_links() {
            match previous {
                Some(p) => {
                    let _ = self.collections.insert(id, p);
                }
                None => {
                    let _ = self.collections.remove(&id);
                }
            }
            return Err(e);
        }
        Ok(())
    }

    /// Removes a collection.
    pub fn remove(&mut self, id: CollectionId) -> Option<Collection> {
        self.collections.remove(&id)
    }

    /// One collection.
    #[must_use]
    pub fn collection(&self, id: CollectionId) -> Option<&Collection> {
        self.collections.get(&id)
    }

    /// All collections.
    pub fn collections(&self) -> impl Iterator<Item = &Collection> {
        self.collections.values()
    }

    /// Replaces several schemas at once, keeping their rows, so a change
    /// that spans collections (both sides of a relation) is checked as a
    /// whole. Nothing changes when the result does not check.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::UnknownCollection`] or the first link error.
    pub fn set_schemas(
        &mut self,
        schemas: Vec<Schema>,
    ) -> Result<(), EditError> {
        let mut previous = Vec::with_capacity(schemas.len());
        for schema in &schemas {
            if !self.collections.contains_key(&schema.id) {
                return Err(EditError::UnknownCollection(schema.id));
            }
        }
        for schema in schemas {
            let id = schema.id;
            let Some(existing) = self.collections.get(&id) else {
                continue;
            };
            let mut next = Collection::new(schema);
            next.rows.clone_from(&existing.rows);
            if let Some(old) = self.collections.insert(id, next) {
                previous.push(old);
            }
        }
        if let Err(e) = self.check_links() {
            for old in previous {
                let _ = self.collections.insert(old.id(), old);
            }
            return Err(e.into());
        }
        Ok(())
    }

    /// Replaces a collection's schema, keeping its rows.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::UnknownCollection`] or the schema's error.
    pub fn set_schema(&mut self, schema: Schema) -> Result<(), EditError> {
        let id = schema.id;
        let existing = self
            .collections
            .get(&id)
            .ok_or(EditError::UnknownCollection(id))?;
        let mut next = Collection::new(schema);
        next.rows.clone_from(&existing.rows);
        self.insert(next)?;
        Ok(())
    }

    pub(crate) fn check_links(&self) -> Result<(), SchemaError> {
        for c in self.collections.values() {
            for p in &c.schema.properties {
                match &p.kind {
                    PropertyType::Relation {
                        to,
                        back: Some(back),
                    } => {
                        let Some(target) = self.collections.get(to) else {
                            continue;
                        };
                        let paired = matches!(
                            target.schema.property(back),
                            Some(Property {
                                kind: PropertyType::Relation {
                                    to: t,
                                    back: Some(b)
                                },
                                ..
                            }) if *t == c.id() && *b == p.key
                        );
                        if !paired {
                            return Err(SchemaError::RelationBack {
                                key: p.key.clone(),
                                back: back.clone(),
                            });
                        }
                    }
                    PropertyType::Rollup { via, prop, .. } => {
                        let to = match c.schema.property(via).map(|v| &v.kind) {
                            Some(PropertyType::Relation { to, .. }) => *to,
                            _ => continue,
                        };
                        let Some(target) = self.collections.get(&to) else {
                            continue;
                        };
                        if target.schema.property(prop).is_none() {
                            return Err(SchemaError::RollupProp {
                                key: p.key.clone(),
                                prop: prop.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Applies an edit.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] and leaves the workspace unchanged when the
    /// edit is invalid.
    pub fn apply(&mut self, edit: Edit) -> Result<Changeset, EditError> {
        match edit {
            Edit::CreateRow {
                collection,
                id,
                values,
                body,
            } => {
                let id = id.unwrap_or_else(DocId::generate);
                let c = self.get(collection)?;
                if c.rows.contains_key(&id) {
                    return Err(EditError::DuplicateRow(id));
                }
                let mut row = Row::new(id);
                row.body = body;
                let typed = self.typed_values(collection, values)?;
                for (key, v) in &typed {
                    row.set_raw(key, v.clone());
                }
                self.replace(collection, row)
            }
            Edit::UpdateRow {
                collection,
                row,
                values,
                body,
            } => {
                let typed = self.typed_values(collection, values)?;
                let mut next = self
                    .get(collection)?
                    .row(row)
                    .cloned()
                    .ok_or(EditError::UnknownRow(row))?;
                for (key, v) in typed {
                    next.set_raw(&key, v);
                }
                if let Some(body) = body {
                    next.body = body;
                }
                self.replace(collection, next)
            }
            Edit::ReplaceRow { collection, row } => {
                let _ = self.get(collection)?;
                let row = self.clean_links(collection, row);
                self.replace(collection, row)
            }
            Edit::DeleteRow { collection, row } => self.delete(collection, row),
        }
    }

    fn get(&self, id: CollectionId) -> Result<&Collection, EditError> {
        self.collections
            .get(&id)
            .ok_or(EditError::UnknownCollection(id))
    }

    fn get_mut(
        &mut self,
        id: CollectionId,
    ) -> Result<&mut Collection, EditError> {
        self.collections
            .get_mut(&id)
            .ok_or(EditError::UnknownCollection(id))
    }

    /// Resolves keys or names to keys and checks each value's type.
    fn typed_values(
        &self,
        collection: CollectionId,
        values: BTreeMap<String, Value>,
    ) -> Result<Vec<(String, Value)>, EditError> {
        let schema = &self.get(collection)?.schema;
        values
            .into_iter()
            .map(|(name, value)| {
                let p = schema
                    .resolve(&name)
                    .ok_or_else(|| EditError::UnknownProperty(name.clone()))?;
                let v = self.coerce(p, value)?;
                Ok((p.key.clone(), v))
            })
            .collect()
    }

    fn coerce(&self, p: &Property, value: Value) -> Result<Value, EditError> {
        let wrong =
            |expected: &'static str, got: &Value| EditError::WrongType {
                key: p.key.clone(),
                expected,
                got: got.type_name(),
            };
        if value.is_empty() {
            return if p.kind.is_stored() {
                Ok(Value::Empty)
            } else {
                Err(EditError::Computed(p.key.clone()))
            };
        }
        let all_text =
            |items: &[Value]| items.iter().all(|i| matches!(i, Value::Text(_)));
        match (&p.kind, value) {
            (
                PropertyType::Title
                | PropertyType::Text
                | PropertyType::Url
                | PropertyType::Select { .. }
                | PropertyType::Person,
                v @ Value::Text(_),
            )
            | (PropertyType::Number { .. }, v @ Value::Number(_))
            | (PropertyType::Checkbox, v @ Value::Bool(_))
            | (PropertyType::Date, v @ Value::Date(_)) => Ok(v),
            (PropertyType::Multi { .. }, Value::Text(t)) => {
                Ok(Value::List(vec![Value::Text(t)]))
            }
            (
                PropertyType::Multi { .. } | PropertyType::Person,
                Value::List(items),
            ) if all_text(&items) => {
                let mut seen = BTreeSet::new();
                Ok(Value::List(
                    items
                        .into_iter()
                        .filter(|i| seen.insert(i.to_string()))
                        .collect(),
                ))
            }
            (
                PropertyType::Relation { to, .. },
                v @ (Value::Text(_) | Value::List(_)),
            ) => {
                let mut ids = Vec::new();
                for item in v.items() {
                    let id: DocId = item
                        .as_text()
                        .and_then(|t| t.parse().ok())
                        .ok_or_else(|| wrong("row ids", item))?;
                    if let Some(target) = self.collections.get(to)
                        && !target.rows.contains_key(&id)
                    {
                        return Err(EditError::UnknownRow(id));
                    }
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                Ok(relation_value(&ids))
            }
            (
                PropertyType::Rollup { .. }
                | PropertyType::Formula { .. }
                | PropertyType::Created,
                _,
            ) => Err(EditError::Computed(p.key.clone())),
            (kind, other) => Err(wrong(expected_shape(kind), &other)),
        }
    }

    /// Drops links to rows that do not exist (from external edits).
    fn clean_links(&self, collection: CollectionId, mut row: Row) -> Row {
        let Some(c) = self.collections.get(&collection) else {
            return row;
        };
        for p in &c.schema.properties {
            if let PropertyType::Relation { to, .. } = &p.kind
                && let Some(target) = self.collections.get(to)
            {
                let kept: Vec<DocId> = row
                    .links(&p.key)
                    .into_iter()
                    .filter(|id| {
                        target.rows.contains_key(id)
                            || (*to == collection && *id == row.id)
                    })
                    .collect();
                row.set_raw(&p.key, relation_value(&kept));
            }
        }
        row
    }

    /// Stores `next` and mirrors relation differences to the other side.
    fn replace(
        &mut self,
        collection: CollectionId,
        next: Row,
    ) -> Result<Changeset, EditError> {
        let c = self.get(collection)?;
        let previous = c.rows.get(&next.id);
        let id = next.id;
        // (target collection, back key, linked row, add?)
        let mut mirror: Vec<(CollectionId, String, DocId, bool)> = Vec::new();
        for p in &c.schema.properties {
            let PropertyType::Relation {
                to,
                back: Some(back),
            } = &p.kind
            else {
                continue;
            };
            let old: BTreeSet<DocId> = previous
                .map(|r| r.links(&p.key).into_iter().collect())
                .unwrap_or_default();
            let new: BTreeSet<DocId> = next.links(&p.key).into_iter().collect();
            for added in new.difference(&old) {
                mirror.push((*to, back.clone(), *added, true));
            }
            for removed in old.difference(&new) {
                mirror.push((*to, back.clone(), *removed, false));
            }
        }
        let mut changes = Changeset::default();
        let _ = changes.written.insert(RowRef {
            collection,
            row: id,
        });
        let _ = self.get_mut(collection)?.rows.insert(id, next);
        for (to, back, row, add) in mirror {
            changes.merge(self.link(to, row, &back, id, add));
        }
        Ok(changes)
    }

    /// Adds or removes `other` in `row`'s relation `key`.
    fn link(
        &mut self,
        collection: CollectionId,
        row: DocId,
        key: &str,
        other: DocId,
        add: bool,
    ) -> Changeset {
        let mut changes = Changeset::default();
        let Some(r) = self
            .collections
            .get_mut(&collection)
            .and_then(|c| c.rows.get_mut(&row))
        else {
            return changes;
        };
        let mut links = r.links(key);
        let had = links.contains(&other);
        if add && !had {
            links.push(other);
        } else if !add && had {
            links.retain(|l| *l != other);
        } else {
            return changes;
        }
        r.set_raw(key, relation_value(&links));
        let _ = changes.written.insert(RowRef { collection, row });
        changes
    }

    fn delete(
        &mut self,
        collection: CollectionId,
        id: DocId,
    ) -> Result<Changeset, EditError> {
        let _ = self.get_mut(collection)?.row_mut(id)?;
        let mut changes = Changeset::default();
        let _ = self.get_mut(collection)?.rows.remove(&id);
        let _ = changes.deleted.insert(RowRef {
            collection,
            row: id,
        });
        // Remove every link to the deleted row, two-way or not.
        for c in self.collections.values_mut() {
            let keys: Vec<String> = c
                .schema
                .properties
                .iter()
                .filter(|p| {
                    matches!(
                        &p.kind,
                        PropertyType::Relation { to, .. } if *to == collection
                    )
                })
                .map(|p| p.key.clone())
                .collect();
            let cid = c.id();
            for row in c.rows.values_mut() {
                for key in &keys {
                    let links = row.links(key);
                    if links.contains(&id) {
                        let kept: Vec<DocId> =
                            links.into_iter().filter(|l| *l != id).collect();
                        row.set_raw(key, relation_value(&kept));
                        let _ = changes.written.insert(RowRef {
                            collection: cid,
                            row: row.id,
                        });
                    }
                }
            }
        }
        Ok(changes)
    }

    /// Rows whose computed values may change after `changes`: the changed
    /// rows plus, transitively, every row that links to one of them from a
    /// collection with rollups.
    #[must_use]
    pub fn affected(&self, changes: &Changeset) -> BTreeSet<RowRef> {
        let mut incoming: BTreeMap<DocId, Vec<RowRef>> = BTreeMap::new();
        for c in self.collections.values() {
            let has_rollups = c
                .schema
                .properties
                .iter()
                .any(|p| matches!(p.kind, PropertyType::Rollup { .. }));
            if !has_rollups {
                continue;
            }
            for p in &c.schema.properties {
                if matches!(p.kind, PropertyType::Relation { .. }) {
                    for row in c.rows.values() {
                        for target in row.links(&p.key) {
                            incoming.entry(target).or_default().push(RowRef {
                                collection: c.id(),
                                row: row.id,
                            });
                        }
                    }
                }
            }
        }
        let mut out = BTreeSet::new();
        let mut queue: VecDeque<RowRef> = changes
            .written
            .iter()
            .chain(&changes.deleted)
            .copied()
            .collect();
        while let Some(r) = queue.pop_front() {
            let exists = self
                .collections
                .get(&r.collection)
                .is_some_and(|c| c.rows.contains_key(&r.row));
            if !out.insert(r) {
                continue;
            }
            if let Some(from) = incoming.get(&r.row) {
                queue.extend(from.iter().copied());
            }
            if !exists {
                let _ = out.remove(&r);
            }
        }
        out
    }
}

const fn expected_shape(kind: &PropertyType) -> &'static str {
    match kind {
        PropertyType::Number { .. } => "a number",
        PropertyType::Checkbox => "true or false",
        PropertyType::Date => "a date",
        PropertyType::Multi { .. } => "a list of options",
        PropertyType::Person => "a person handle or a list of them",
        PropertyType::Relation { .. } => "row ids",
        _ => "text",
    }
}
