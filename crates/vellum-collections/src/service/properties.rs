// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Adding, changing and removing a collection's properties (ADR 0045).
//!
//! A change may span collections: a relation has a paired property in its
//! target. Every changed schema is checked together, exactly as files
//! loaded from disk would be, and then written with the same etag guard as
//! saved views; a failed write puts back the ones already written. Row
//! files are not rewritten: after a type change the rows are read again,
//! and a value that no longer fits is kept as written and reported.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tracing::Instrument as _;
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope};
use vellum_kernel::{CollectionId, Error, ErrorKind};

use super::state::State;
use super::{
    AddPropertyInput, CollectionInfo, CollectionsService, PropertyKey, UpdatePropertyInput,
};
use crate::schema::{Property, PropertySpec, PropertyType, Schema, SelectOption};

/// Longest property name, in characters.
pub const MAX_PROPERTY_NAME_CHARS: usize = 100;

/// Every loaded schema, by collection, while a change is worked out.
type Schemas = BTreeMap<CollectionId, Schema>;

/// Collections whose rows must be read again after the change.
type Reparse = BTreeSet<CollectionId>;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

fn not_found(id: CollectionId) -> Error {
    Error::new(ErrorKind::NotFound, format!("collection {id} not found"))
}

fn property_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_PROPERTY_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "a property name needs 1 to {MAX_PROPERTY_NAME_CHARS} printable characters"
        )));
    }
    Ok(name.to_owned())
}

fn schema_mut(schemas: &mut Schemas, id: CollectionId) -> Result<&mut Schema, Error> {
    schemas.get_mut(&id).ok_or_else(|| not_found(id))
}

fn index_of(schema: &Schema, key: &str) -> Result<usize, Error> {
    schema
        .properties
        .iter()
        .position(|p| p.key == key)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("no property with key {key:?}")))
}

fn name_taken(schema: &Schema, name: &str, except: Option<&str>) -> Result<(), Error> {
    let lower = name.to_lowercase();
    let taken = schema
        .properties
        .iter()
        .any(|p| Some(p.key.as_str()) != except && p.name.to_lowercase() == lower);
    if taken {
        return Err(Error::new(
            ErrorKind::Conflict,
            format!("a property named {name:?} already exists"),
        ));
    }
    Ok(())
}

/// A fresh key and a free name (suffixed if taken) for a paired property.
fn free_key_and_name(schema: &Schema, name: &str, also: &[&Property]) -> (String, String) {
    let mut keys: BTreeSet<String> = schema.properties.iter().map(|p| p.key.clone()).collect();
    let mut names: BTreeSet<String> = schema
        .properties
        .iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    for p in also {
        let _ = keys.insert(p.key.clone());
        let _ = names.insert(p.name.to_lowercase());
    }
    let index = schema.properties.len() + also.len();
    let key = crate::csv::unique_key(name, index, &mut keys);
    let name = crate::csv::unique_name(name, index, &mut names);
    (key, name)
}

/// The names of the other properties that read `key`: formulas name it,
/// rollups follow it.
fn users_of(schema: &Schema, key: &str) -> Vec<String> {
    schema
        .properties
        .iter()
        .filter(|p| p.key != key)
        .filter(|p| match &p.kind {
            PropertyType::Formula { formula, .. } => formula.references().contains(key),
            PropertyType::Rollup { via, .. } => via == key,
            _ => false,
        })
        .map(|p| p.name.clone())
        .collect()
}

fn in_use(what: &str, name: &str, users: &[String]) -> Error {
    Error::new(
        ErrorKind::Conflict,
        format!(
            "cannot {what} {name:?}: {} use{} it; change {} first",
            users.join(", "),
            if users.len() == 1 { "s" } else { "" },
            if users.len() == 1 { "it" } else { "them" },
        ),
    )
}

/// A rollup's relation and target property, as keys: `via` is looked up
/// here, `prop` in the collection the relation links to.
fn rollup_keys(
    schemas: &Schemas,
    id: CollectionId,
    via: Option<&str>,
    prop: Option<&str>,
) -> Result<(Option<String>, Option<String>), Error> {
    let (Some(via), Some(prop)) = (via, prop) else {
        return Ok((via.map(str::to_owned), prop.map(str::to_owned)));
    };
    let schema = schemas.get(&id).ok_or_else(|| not_found(id))?;
    let relation = schema
        .resolve(via)
        .ok_or_else(|| invalid(format!("no relation named {via:?}")))?;
    let PropertyType::Relation { to, .. } = relation.kind else {
        return Err(invalid(format!("{} is not a relation", relation.name)));
    };
    let target = schemas
        .get(&to)
        .and_then(|t| t.resolve(prop))
        .ok_or_else(|| invalid(format!("the linked collection has no property {prop:?}")))?;
    Ok((Some(relation.key.clone()), Some(target.key.clone())))
}

/// Adds a two-way relation from `id` to `input.to`, with its pair.
fn add_relation(
    schemas: &mut Schemas,
    id: CollectionId,
    input: &AddPropertyInput,
    key: String,
    name: String,
) -> Result<(), Error> {
    let to = input
        .to
        .ok_or_else(|| invalid("a relation needs the collection it links to"))?;
    let source_title = schemas
        .get(&id)
        .map(|s| s.title.clone())
        .unwrap_or_default();
    let wanted = match input.back_name.as_deref().map(str::trim) {
        Some(n) if !n.is_empty() => property_name(n)?,
        _ if source_title.trim().is_empty() => "Related".to_owned(),
        _ => source_title,
    };
    let forward = Property {
        key: key.clone(),
        name,
        kind: PropertyType::Relation { to, back: None },
    };
    let target = schemas.get(&to).ok_or_else(|| not_found(to))?;
    let also: Vec<&Property> = if to == id { vec![&forward] } else { Vec::new() };
    let (back_key, back_name) = free_key_and_name(target, &wanted, &also);
    let back = Property {
        key: back_key.clone(),
        name: back_name,
        kind: PropertyType::Relation {
            to: id,
            back: Some(key),
        },
    };
    let forward = Property {
        kind: PropertyType::Relation {
            to,
            back: Some(back_key),
        },
        ..forward
    };
    schema_mut(schemas, id)?.properties.push(forward);
    schema_mut(schemas, to)?.properties.push(back);
    Ok(())
}

/// Adds the property `input` describes, keyed from its name.
fn add_property(
    schemas: &mut Schemas,
    id: CollectionId,
    input: &AddPropertyInput,
) -> Result<(), Error> {
    let schema = schemas.get(&id).ok_or_else(|| not_found(id))?;
    if schema.properties.len() >= crate::schema::MAX_PROPERTIES {
        return Err(invalid(format!(
            "a collection may have at most {} properties",
            crate::schema::MAX_PROPERTIES
        )));
    }
    let name = property_name(&input.name)?;
    name_taken(schema, &name, None)?;
    if input.kind == "title" {
        return Err(invalid("a collection has exactly one title property"));
    }
    let mut used: BTreeSet<String> = schema.properties.iter().map(|p| p.key.clone()).collect();
    let key = crate::csv::unique_key(&name, schema.properties.len(), &mut used);
    if input.kind == "relation" {
        return add_relation(schemas, id, input, key, name);
    }
    let (via, prop) = rollup_keys(schemas, id, input.via.as_deref(), input.prop.as_deref())?;
    let spec = PropertySpec {
        key: key.clone(),
        name: name.clone(),
        kind: input.kind.clone(),
        options: input.options.clone(),
        via,
        prop,
        agg: input.agg,
        formula: input.formula.clone(),
        format: input.format,
    };
    let property = Property {
        key,
        name,
        kind: Schema::property_type(&spec)?,
    };
    schema_mut(schemas, id)?.properties.push(property);
    Ok(())
}

/// Renames the property at `at`, rewriting the formulas that name it.
fn rename(schema: &mut Schema, at: usize, key: &str, name: &str) -> Result<(), Error> {
    let name = property_name(name)?;
    name_taken(schema, &name, Some(key))?;
    let before = schema.clone();
    for p in &mut schema.properties {
        if let PropertyType::Formula { formula, .. } = &mut p.kind
            && formula.references().contains(key)
        {
            let source = crate::formula::rename_references(formula.source(), &before, key, &name)
                .map_err(|e| invalid(format!("{}: {}", p.name, e.message)))?;
            *formula = crate::formula::Formula::uncompiled(source);
        }
    }
    if let Some(p) = schema.properties.get_mut(at) {
        p.name = name;
    }
    Ok(())
}

/// The options to keep: trimmed, unique, non-empty.
fn clean_options(options: &[SelectOption]) -> Vec<SelectOption> {
    let mut seen = BTreeSet::new();
    options
        .iter()
        .filter(|o| !o.name.trim().is_empty() && seen.insert(o.name.trim().to_owned()))
        .map(|o| SelectOption {
            name: o.name.trim().to_owned(),
            color: o.color.clone(),
        })
        .collect()
}

/// Gives the property at `at` a new type, carrying over what still applies.
fn retype(
    schemas: &mut Schemas,
    id: CollectionId,
    at: usize,
    input: &UpdatePropertyInput,
    kind: &str,
) -> Result<(), Error> {
    let (via, prop) = rollup_keys(schemas, id, input.via.as_deref(), input.prop.as_deref())?;
    let schema = schema_mut(schemas, id)?;
    let Some(p) = schema.properties.get(at) else {
        return Err(Error::new(ErrorKind::NotFound, "no such property"));
    };
    let fixed = |k: &str| matches!(k, "title" | "relation");
    if fixed(p.kind.name()) || fixed(kind) {
        return Err(invalid(format!(
            "{} cannot become a {kind}: titles and relations keep their type",
            p.name
        )));
    }
    let formula = match &p.kind {
        PropertyType::Formula { formula, .. } => Some(formula.source().to_owned()),
        _ => None,
    };
    let spec = PropertySpec {
        key: p.key.clone(),
        name: p.name.clone(),
        kind: kind.to_owned(),
        options: input
            .options
            .as_deref()
            .map_or_else(|| p.kind.options().to_vec(), clean_options),
        via,
        prop,
        agg: input.agg,
        formula: input.formula.clone().or(formula),
        format: input.format,
    };
    let kind = Schema::property_type(&spec)?;
    if let Some(p) = schema.properties.get_mut(at) {
        p.kind = kind;
    }
    Ok(())
}

/// Changes options, a formula or a number format in place.
fn change_settings(p: &mut Property, input: &UpdatePropertyInput) -> Result<(), Error> {
    let wrong = |field: &str, p: &Property| {
        invalid(format!(
            "{field} does not apply to {} ({} property)",
            p.name,
            p.kind.name()
        ))
    };
    if let Some(new) = &input.options {
        match &mut p.kind {
            PropertyType::Select { options } | PropertyType::Multi { options } => {
                *options = clean_options(new);
            }
            _ => return Err(wrong("options", p)),
        }
    }
    if let Some(source) = &input.formula {
        match &mut p.kind {
            PropertyType::Formula { formula, .. } => {
                *formula = crate::formula::Formula::uncompiled(source.clone());
            }
            _ => return Err(wrong("a formula", p)),
        }
    }
    if let Some(new) = input.format {
        match &mut p.kind {
            PropertyType::Number { format }
            | PropertyType::Formula { format, .. }
            | PropertyType::Rollup { format, .. } => *format = new,
            _ => return Err(wrong("a number format", p)),
        }
    }
    Ok(())
}

/// Applies `input`; returns whether the rows must be read again.
fn change_property(
    schemas: &mut Schemas,
    id: CollectionId,
    input: &UpdatePropertyInput,
) -> Result<bool, Error> {
    let key = input.key.as_str();
    let at = index_of(schema_mut(schemas, id)?, key)?;
    if let Some(name) = &input.name {
        rename(schema_mut(schemas, id)?, at, key, name)?;
    }
    let current = schema_mut(schemas, id)?
        .properties
        .get(at)
        .map(|p| p.kind.name())
        .unwrap_or_default();
    if let Some(kind) = input.kind.as_deref().filter(|k| *k != current) {
        retype(schemas, id, at, input, kind)?;
        return Ok(true);
    }
    let p = schema_mut(schemas, id)?
        .properties
        .get_mut(at)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no such property"))?;
    change_settings(p, input)?;
    Ok(false)
}

/// Removes `key` and every view setting that names it.
fn strip(schema: &mut Schema, key: &str) -> Result<(), Error> {
    let at = index_of(schema, key)?;
    let users = users_of(schema, key);
    if let Some(p) = schema.properties.get(at)
        && !users.is_empty()
    {
        return Err(in_use("remove", &p.name, &users));
    }
    let _ = schema.properties.remove(at);
    for view in &mut schema.views {
        view.filters.retain(|f| f.key != key);
        view.sorts.retain(|s| s.key != key);
        view.hidden.retain(|h| h != key);
        for slot in [&mut view.group, &mut view.date, &mut view.end] {
            if slot.as_deref() == Some(key) {
                *slot = None;
            }
        }
    }
    Ok(())
}

/// Removes a property; a two-way relation takes its pair with it.
fn remove_property(schemas: &mut Schemas, id: CollectionId, key: &str) -> Result<(), Error> {
    let schema = schema_mut(schemas, id)?;
    let p = schema
        .properties
        .get(index_of(schema, key)?)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no such property"))?;
    let pair = match &p.kind {
        PropertyType::Title => return Err(invalid("the title property cannot be removed")),
        PropertyType::Relation { to, back: Some(b) } => Some((*to, b.clone())),
        _ => None,
    };
    strip(schema, key)?;
    if let Some((to, back)) = pair
        && let Some(target) = schemas.get_mut(&to)
        && target.property(&back).is_some()
    {
        strip(target, &back)?;
    }
    Ok(())
}

/// Swaps in the checked version of each edited schema.
fn checked(schemas: &mut Schemas, ids: &[CollectionId]) -> Result<(), Error> {
    for id in ids {
        let schema = schema_mut(schemas, *id)?;
        *schema = schema.revalidate()?;
    }
    Ok(())
}

/// The collection as it now stands.
fn info_of(state: &State, id: CollectionId) -> Result<CollectionInfo, Error> {
    match (state.ws.collection(id), state.folder(id)) {
        (Some(c), Ok(f)) => Ok(CollectionInfo::new(
            c.schema(),
            f.path.as_str().to_owned(),
            f.diagnostics.clone(),
        )),
        _ => Err(Error::new(ErrorKind::Internal, "collection vanished")),
    }
}

impl CollectionsService {
    /// Changes one or more schemas under the write lock: `change` edits a
    /// copy of every schema, the changed ones are checked together, the
    /// caller must be able to edit each, and they are written in turn.
    async fn edit_schemas<F>(
        &self,
        ctx: &RequestContext,
        op: &'static str,
        id: CollectionId,
        change: F,
    ) -> Result<CollectionInfo, Error>
    where
        F: FnOnce(&mut Schemas) -> Result<Reparse, Error> + Send,
    {
        let span = tracing::info_span!("collections.edit_schema", op, collection = %id);
        async move {
            let mut guard = self.state.write().await;
            self.bump();
            if guard.is_none() {
                *guard = Some(State::load(self.store.as_ref()).await?);
            }
            let Some(state) = guard.as_mut() else {
                return Err(Error::new(
                    ErrorKind::Internal,
                    "collections were not loaded",
                ));
            };
            self.require_edit(state, &ctx.principal, id, false).await?;
            let before: Schemas = state
                .ws
                .collections()
                .map(|c| (c.id(), c.schema().clone()))
                .collect();
            let mut after = before.clone();
            let reparse = change(&mut after)?;
            let text = |s: &Schema| s.to_yaml().ok();
            let changed: Vec<CollectionId> = after
                .iter()
                .filter(|(cid, s)| before.get(cid).map(text) != Some(text(s)))
                .map(|(cid, _)| *cid)
                .collect();
            checked(&mut after, &changed)?;
            for other in changed.iter().filter(|c| **c != id) {
                self.require_edit(state, &ctx.principal, *other, false)
                    .await?;
            }
            let next: Vec<Schema> = changed.iter().filter_map(|c| after.remove(c)).collect();
            state.ws.set_schemas(next)?;
            let mut written = Vec::new();
            for cid in &changed {
                if let Err(e) = state.write_schema(self.store.as_ref(), *cid).await {
                    crate::telemetry::wrote(op, false);
                    // Put back what was already written, so a pair stays whole.
                    let old: Vec<Schema> = changed
                        .iter()
                        .filter_map(|c| before.get(c).cloned())
                        .collect();
                    if state.ws.set_schemas(old).is_ok() {
                        for done in &written {
                            let _ = state.write_schema(self.store.as_ref(), *done).await;
                        }
                    }
                    *guard = None;
                    return Err(e);
                }
                written.push(*cid);
            }
            crate::telemetry::wrote(op, true);
            let dirs: BTreeSet<_> = reparse
                .iter()
                .filter_map(|c| state.folder(*c).ok().map(|f| f.path.clone()))
                .collect();
            if !dirs.is_empty() && state.reload(self.store.as_ref(), &dirs).await.is_err() {
                *guard = Some(State::load(self.store.as_ref()).await?);
            }
            let info = guard
                .as_ref()
                .ok_or_else(|| Error::new(ErrorKind::Internal, "collections were not loaded"))
                .and_then(|state| info_of(state, id));
            drop(guard);
            info
        }
        .instrument(span)
        .await
    }

    /// Adds a property at the end of the column order. A relation also
    /// adds its pair to the collection it links to, which needs edit
    /// access there.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a bad name, type, formula or rollup;
    /// `conflict` when the name is taken; `permission_denied` without
    /// edit access.
    pub async fn add_property(
        &self,
        ctx: &RequestContext,
        input: AddPropertyInput,
    ) -> Result<CollectionInfo, Error> {
        let id = input.collection_id;
        self.edit_schemas(ctx, "add_property", id, move |schemas| {
            add_property(schemas, id, &input)?;
            Ok(Reparse::new())
        })
        .await
    }

    /// Renames a property (rewriting the formulas that name it), changes
    /// its type, or changes its options, formula or format.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown key; `conflict` when the new name is
    /// taken; `invalid_input` for a setting the type does not have or a
    /// type a title or relation cannot take.
    pub async fn update_property(
        &self,
        ctx: &RequestContext,
        input: UpdatePropertyInput,
    ) -> Result<CollectionInfo, Error> {
        let id = input.collection_id;
        self.edit_schemas(ctx, "update_property", id, move |schemas| {
            let retyped = change_property(schemas, id, &input)?;
            Ok(if retyped {
                Reparse::from([id])
            } else {
                Reparse::new()
            })
        })
        .await
    }

    /// Removes a property, and the filters, sorts and grouping that use
    /// it. A two-way relation is removed from both collections.
    ///
    /// # Errors
    ///
    /// `invalid_input` for the title; `conflict` while a formula or rollup
    /// reads it.
    pub async fn delete_property(
        &self,
        ctx: &RequestContext,
        key: PropertyKey,
    ) -> Result<CollectionInfo, Error> {
        let id = key.collection_id;
        self.edit_schemas(ctx, "delete_property", id, move |schemas| {
            remove_property(schemas, id, &key.key)?;
            Ok(Reparse::new())
        })
        .await
    }

    pub(super) fn property_operations(this: &Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(this);
        let add = Operation::mutation(
            "collections.add_property",
            HttpMethod::Post,
            "/v1/collections/{collection_id}/properties",
            Scope::WriteDirect,
            move |ctx, input: AddPropertyInput| {
                let s = Arc::clone(&s);
                async move { s.add_property(&ctx, input).await }
            },
        )
        .summary("Add a property (a column) to a collection");
        let s = Arc::clone(this);
        let update = Operation::mutation(
            "collections.update_property",
            HttpMethod::Patch,
            "/v1/collections/{collection_id}/properties/{key}",
            Scope::WriteDirect,
            move |ctx, input: UpdatePropertyInput| {
                let s = Arc::clone(&s);
                async move { s.update_property(&ctx, input).await }
            },
        )
        .summary("Rename a property or change its type, options, formula or format");
        let s = Arc::clone(this);
        let delete = Operation::mutation(
            "collections.delete_property",
            HttpMethod::Delete,
            "/v1/collections/{collection_id}/properties/{key}",
            Scope::WriteDirect,
            move |ctx, key: PropertyKey| {
                let s = Arc::clone(&s);
                async move { s.delete_property(&ctx, key).await }
            },
        )
        .summary("Remove a property and the view settings that use it");
        vec![add, update, delete]
    }
}
