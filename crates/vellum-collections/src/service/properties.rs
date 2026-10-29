// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Adding, changing and removing a collection's properties (ADR 0045).
//!
//! Each change goes through the same etag-guarded `schema.yaml` rewrite as
//! saved views, and the changed schema is checked exactly as a file loaded
//! from disk would be. Row files are not rewritten: a removed property's
//! values stay in their front matter until the row is next saved.

use std::collections::BTreeSet;
use std::sync::Arc;

use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope};
use vellum_kernel::{Error, ErrorKind};

use super::{
    AddPropertyInput, CollectionInfo, CollectionsService, PropertyKey, UpdatePropertyInput,
};
use crate::schema::{Property, PropertySpec, PropertyType, Schema};

/// Longest property name, in characters.
pub const MAX_PROPERTY_NAME_CHARS: usize = 100;

fn property_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_PROPERTY_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("a property name needs 1 to {MAX_PROPERTY_NAME_CHARS} printable characters"),
        ));
    }
    Ok(name.to_owned())
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

/// The names of the other properties that read `key`: formulas name it,
/// rollups follow it (unless `formulas_only`).
fn users_of(schema: &Schema, key: &str, formulas_only: bool) -> Vec<String> {
    schema
        .properties
        .iter()
        .filter(|p| p.key != key)
        .filter(|p| match &p.kind {
            PropertyType::Formula { formula, .. } => formula.references().contains(key),
            PropertyType::Rollup { via, .. } => !formulas_only && via == key,
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

/// Builds the new property for `input`, keyed from its name.
fn new_property(schema: &Schema, input: &AddPropertyInput) -> Result<Property, Error> {
    let name = property_name(&input.name)?;
    name_taken(schema, &name, None)?;
    match input.kind.as_str() {
        "title" => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "a collection has exactly one title property",
            ));
        }
        "relation" => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "relations link two collections; add them in schema.yaml",
            ));
        }
        _ => {}
    }
    let mut used: BTreeSet<String> = schema.properties.iter().map(|p| p.key.clone()).collect();
    let key = crate::csv::unique_key(&name, schema.properties.len(), &mut used);
    // A rollup's relation may be named; the file stores its key.
    let via = input.via.as_deref().map(|v| {
        schema
            .resolve(v)
            .map_or_else(|| v.to_owned(), |p| p.key.clone())
    });
    let spec = PropertySpec {
        key: key.clone(),
        name: name.clone(),
        kind: input.kind.clone(),
        options: input.options.clone(),
        via,
        prop: input.prop.clone(),
        agg: input.agg,
        formula: input.formula.clone(),
        format: input.format,
    };
    Ok(Property {
        key,
        name,
        kind: Schema::property_type(&spec)?,
    })
}

/// Applies `input` to the property at `at`.
fn change_property(
    schema: &mut Schema,
    at: usize,
    input: &UpdatePropertyInput,
) -> Result<(), Error> {
    let key = input.key.clone();
    if let Some(name) = &input.name {
        let name = property_name(name)?;
        name_taken(schema, &name, Some(&key))?;
        let old = schema
            .properties
            .get(at)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        // Formulas name properties in their source, so a rename would break them.
        let formulas = users_of(schema, &key, true);
        if old != name && !formulas.is_empty() {
            return Err(in_use("rename", &old, &formulas));
        }
        if let Some(p) = schema.properties.get_mut(at) {
            p.name = name;
        }
    }
    let Some(p) = schema.properties.get_mut(at) else {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("no property with key {key:?}"),
        ));
    };
    let wrong = |field: &str, p: &Property| {
        Error::new(
            ErrorKind::InvalidInput,
            format!(
                "{field} does not apply to {} ({} property)",
                p.name,
                p.kind.name()
            ),
        )
    };
    if let Some(new) = &input.options {
        let mut seen = BTreeSet::new();
        let new: Vec<_> = new
            .iter()
            .filter(|o| !o.name.trim().is_empty() && seen.insert(o.name.trim().to_owned()))
            .map(|o| crate::schema::SelectOption {
                name: o.name.trim().to_owned(),
                color: o.color.clone(),
            })
            .collect();
        match &mut p.kind {
            PropertyType::Select { options } | PropertyType::Multi { options } => *options = new,
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

/// Removes the property at `at` and every view setting that names it.
fn remove_property(schema: &mut Schema, at: usize) -> Result<(), Error> {
    let Some(p) = schema.properties.get(at) else {
        return Err(Error::new(ErrorKind::NotFound, "no such property"));
    };
    match &p.kind {
        PropertyType::Title => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the title property cannot be removed",
            ));
        }
        PropertyType::Relation { back: Some(_), .. } => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "a two-way relation is removed from both collections' schema.yaml",
            ));
        }
        _ => {}
    }
    let users = users_of(schema, &p.key, false);
    if !users.is_empty() {
        return Err(in_use("remove", &p.name, &users));
    }
    let key = schema.properties.remove(at).key;
    for view in &mut schema.views {
        view.filters.retain(|f| f.key != key);
        view.sorts.retain(|s| s.key != key);
        view.hidden.retain(|h| *h != key);
        for slot in [&mut view.group, &mut view.date, &mut view.end] {
            if slot.as_deref() == Some(key.as_str()) {
                *slot = None;
            }
        }
    }
    Ok(())
}

/// Swaps in the checked version of an edited schema.
fn checked(schema: &mut Schema) -> Result<(), Error> {
    *schema = schema.revalidate()?;
    Ok(())
}

impl CollectionsService {
    /// Adds a property at the end of the column order.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a bad name, type or formula; `conflict` when
    /// the name is taken; `permission_denied` without edit access.
    pub async fn add_property(
        &self,
        ctx: &RequestContext,
        input: AddPropertyInput,
    ) -> Result<CollectionInfo, Error> {
        self.edit_schema(ctx, "add_property", input.collection_id, move |schema| {
            if schema.properties.len() >= crate::schema::MAX_PROPERTIES {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "a collection may have at most {} properties",
                        crate::schema::MAX_PROPERTIES
                    ),
                ));
            }
            let property = new_property(schema, &input)?;
            schema.properties.push(property);
            checked(schema)
        })
        .await
    }

    /// Renames a property, or changes its options, formula or format.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown key; `conflict` when the new name is
    /// taken or a formula names the property; `invalid_input` for a
    /// setting the type does not have.
    pub async fn update_property(
        &self,
        ctx: &RequestContext,
        input: UpdatePropertyInput,
    ) -> Result<CollectionInfo, Error> {
        self.edit_schema(ctx, "update_property", input.collection_id, move |schema| {
            let at = index_of(schema, &input.key)?;
            change_property(schema, at, &input)?;
            checked(schema)
        })
        .await
    }

    /// Removes a property, and the filters, sorts and grouping that use it.
    ///
    /// # Errors
    ///
    /// `invalid_input` for the title or a two-way relation; `conflict`
    /// while a formula or rollup reads it.
    pub async fn delete_property(
        &self,
        ctx: &RequestContext,
        key: PropertyKey,
    ) -> Result<CollectionInfo, Error> {
        self.edit_schema(ctx, "delete_property", key.collection_id, move |schema| {
            let at = index_of(schema, &key.key)?;
            remove_property(schema, at)?;
            checked(schema)
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
        .summary("Rename a property or change its options, formula or format");
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
