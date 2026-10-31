// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The OpenAPI 3.1 document, generated from the registered operations.
//!
//! OpenAPI 3.1 uses JSON Schema 2020-12, the dialect `schemars` emits, so
//! operation schemas go in unchanged. Each operation's named types are
//! lifted into `components/schemas`; when two services use the same type
//! name for different shapes, the later one is renamed after its operation.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use vellum_contracts::{ApiError, HttpMethod, OperationKind};

use crate::registry::{Registry, graphql_name};

/// The `components/schemas` name of the shared error body.
pub const ERROR_SCHEMA: &str = "ApiError";

fn pascal(id: &str) -> String {
    let name = graphql_name(id);
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

/// Replaces `#/$defs/<name>` references using `names`. A bare `#` is the
/// root schema itself (a recursive type such as a task's subtasks), stored
/// under `root`.
fn rewrite_refs(
    value: &Value,
    names: &BTreeMap<String, String>,
    root: &str,
) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    if let Some(name) = v
                        .as_str()
                        .and_then(|r| {
                            if r == "#" {
                                Some(root)
                            } else {
                                r.strip_prefix("#/$defs/")
                            }
                        })
                        .filter(|_| k == "$ref")
                    {
                        let target =
                            names.get(name).map_or(name, String::as_str);
                        return (
                            k.clone(),
                            Value::String(format!(
                                "#/components/schemas/{target}"
                            )),
                        );
                    }
                    (k.clone(), rewrite_refs(v, names, root))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items.iter().map(|v| rewrite_refs(v, names, root)).collect(),
        ),
        other => other.clone(),
    }
}

/// Collects shared schemas under unique names.
#[derive(Debug, Default)]
pub struct Components {
    schemas: BTreeMap<String, Value>,
}

impl Components {
    /// Adds a root schema from `schemars` and its `$defs`, returning a
    /// schema for the root that references the components. The root is
    /// stored under its `title` (or `fallback`).
    pub fn add_root(
        &mut self,
        root: &Value,
        fallback: &str,
        owner: &str,
    ) -> Value {
        let mut root = root.as_object().cloned().unwrap_or_default();
        let defs = root
            .remove("$defs")
            .and_then(|d| d.as_object().cloned())
            .unwrap_or_default();
        let _ = root.remove("$schema");
        let title = root
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| {
                t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .map_or_else(|| fallback.to_owned(), str::to_owned);
        let mut all: Map<String, Value> = defs;
        let _ = all.insert(title.clone(), Value::Object(root));
        let mut names: BTreeMap<String, String> =
            all.keys().map(|k| (k.clone(), k.clone())).collect();
        // Rename until every name is free or holds the identical schema.
        loop {
            let mut changed = false;
            for (name, schema) in &all {
                let rewritten = rewrite_refs(schema, &names, &title);
                let current =
                    names.get(name).cloned().unwrap_or_else(|| name.clone());
                if self
                    .schemas
                    .get(&current)
                    .is_some_and(|existing| *existing != rewritten)
                {
                    let mut candidate = format!("{owner}{name}");
                    let mut n = 2;
                    while self.schemas.contains_key(&candidate)
                        || names.values().any(|v| *v == candidate)
                    {
                        candidate = format!("{owner}{name}{n}");
                        n += 1;
                    }
                    let _ = names.insert(name.clone(), candidate);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (name, schema) in &all {
            let target =
                names.get(name).cloned().unwrap_or_else(|| name.clone());
            let rewritten = rewrite_refs(schema, &names, &title);
            let _ = self.schemas.entry(target).or_insert(rewritten);
        }
        let target = names.get(&title).cloned().unwrap_or(title);
        json!({ "$ref": format!("#/components/schemas/{target}") })
    }

    /// The collected schemas.
    #[must_use]
    pub const fn schemas(&self) -> &BTreeMap<String, Value> {
        &self.schemas
    }

    /// Looks up a component by `$ref`.
    #[must_use]
    pub fn resolve<'a>(&'a self, schema: &'a Value) -> &'a Value {
        schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.strip_prefix("#/components/schemas/"))
            .and_then(|name| self.schemas.get(name))
            .unwrap_or(schema)
    }
}

fn parameter(
    name: &str,
    location: &str,
    required: bool,
    schema: &Value,
) -> Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "schema": schema
    })
}

/// Builds the OpenAPI 3.1 document for every registered operation.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one linear description of the document"
)]
pub fn document(registry: &Registry, version: &str) -> Value {
    let mut components = Components::default();
    let error = schemars::SchemaGenerator::new(
        schemars::generate::SchemaSettings::draft2020_12(),
    )
    .into_root_schema_for::<ApiError>();
    let error_ref =
        components.add_root(&Value::from(error), ERROR_SCHEMA, "Gateway");
    let mut paths: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut tags = std::collections::BTreeSet::new();
    for registered in registry.operations() {
        let op = &registered.op;
        let owner = pascal(op.id());
        let input_ref = components.add_root(
            &Value::from(op.input_schema().clone()),
            &format!("{owner}Input"),
            &owner,
        );
        let output_ref = components.add_root(
            &Value::from(op.output_schema().clone()),
            &format!("{owner}Output"),
            &owner,
        );
        let input = components.resolve(&input_ref).clone();
        let properties = input
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let required: Vec<&str> = input
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let path_params: Vec<&str> = op.path_params().collect();
        let string_schema = json!({"type": "string"});
        let mut parameters: Vec<Value> = path_params
            .iter()
            .map(|p| {
                parameter(
                    p,
                    "path",
                    true,
                    properties.get(*p).unwrap_or(&string_schema),
                )
            })
            .collect();
        let mut operation = Map::new();
        if op.method().has_body() {
            let _ = operation.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": { "application/json": { "schema": input_ref } }
                }),
            );
        } else {
            for (name, schema) in &properties {
                if !path_params.contains(&name.as_str()) {
                    parameters.push(parameter(
                        name,
                        "query",
                        required.contains(&name.as_str()),
                        schema,
                    ));
                }
            }
        }
        if op.kind() == OperationKind::Mutation {
            parameters.push(json!({
                "name": "Idempotency-Key", "in": "header", "required": false,
                "description": "A unique key that makes retries of this write \
                    safe.",
                "schema": {
                    "type": "string",
                    "maxLength": crate::idempotency::MAX_KEY_LEN
                }
            }));
        }
        let security = if op.allows_anonymous() {
            json!([{}, { "session": [] }, { "bearer": [op.scope().as_str()] }])
        } else {
            json!([{ "session": [] }, { "bearer": [op.scope().as_str()] }])
        };
        let _ = tags.insert(registered.service);
        for (key, value) in [
            ("operationId", json!(graphql_name(op.id()))),
            ("summary", json!(op.summary_text())),
            ("description", json!(op.description_text())),
            ("tags", json!([registered.service])),
            ("deprecated", json!(op.is_deprecated())),
            ("parameters", Value::Array(parameters)),
            ("security", security),
            ("x-vellum-operation", json!(op.id())),
            ("x-vellum-input", input_ref.clone()),
            ("x-vellum-kind", json!(op.kind())),
            ("x-vellum-scope", json!(op.scope().as_str())),
            ("x-vellum-budget-ms", json!(op.time_budget().as_millis())),
            (
                "responses",
                json!({
                    "200": { "description": "Success", "content": { "applicatio\
                        n/json": { "schema": output_ref } } },
                    "default": { "$ref": "#/components/responses/Error" }
                }),
            ),
        ] {
            let _ = operation.insert(key.into(), value);
        }
        if let Some(window) = op.fresh_auth_window() {
            let _ = operation.insert(
                "x-vellum-fresh-auth-seconds".into(),
                json!(window.as_secs()),
            );
        }
        let method = match op.method() {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Patch => "patch",
            HttpMethod::Delete => "delete",
        };
        let _ = paths
            .entry(op.path().to_owned())
            .or_default()
            .insert(method.into(), Value::Object(operation));
    }
    json!({
        "openapi": "3.1.1",
        "info": {
            "title": "Vellum API",
            "version": version,
            "summary": "Every Vellum service over REST. The same operations \
                are served over GraphQL at /api/graphql.",
            "license": {
                "name": "AGPL-3.0-only",
                "identifier": "AGPL-3.0-only"
            }
        },
        "jsonSchemaDialect": "https://json-schema.org/draft/2020-12/schema",
        "servers": [{ "url": "/api" }],
        "tags": tags.iter().map(|t| json!({ "name": t })).collect::<Vec<_>>(),
        "paths": paths,
        "components": {
            "schemas": components.schemas(),
            "responses": {
                "Error": {
                    "description": "An error. `code` is stable; `message` is \
                        safe to show; quote `request_id` when reporting it.",
                    "headers": {
                        "X-Request-Id": { "schema": { "type": "string" } }
                    },
                    "content": { "application/json": { "schema": error_ref } }
                }
            },
            "securitySchemes": {
                "session": {
                    "type": "apiKey", "in": "cookie", "name": "vellum_session",
                    "description": "The browser session. Unsafe requests also \
                        need the X-CSRF-Token header."
                },
                "bearer": {
                    "type": "http", "scheme": "bearer",
                    "description": "A scoped API token or an OAuth 2.1 access \
                        token."
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_dedupe_and_rename() {
        let mut c = Components::default();
        let a = json!({"title": "Page", "type": "object", "properties": {
            "x": {"$ref": "#/$defs/Kind"}
        },
            "$defs": {"Kind": {"type": "string", "enum": ["a"]}}});
        let b = json!({"title": "Page", "type": "object", "properties": {
            "x": {"$ref": "#/$defs/Kind"}
        },
            "$defs": {"Kind": {"type": "string", "enum": ["b"]}}});
        assert_eq!(
            c.add_root(&a, "F", "One"),
            json!({"$ref": "#/components/schemas/Page"})
        );
        assert_eq!(
            c.add_root(&a, "F", "Two"),
            json!({"$ref": "#/components/schemas/Page"})
        );
        assert_eq!(
            c.add_root(&b, "F", "Two"),
            json!({"$ref": "#/components/schemas/TwoPage"})
        );
        let two = c.schemas().get("TwoPage").cloned().unwrap_or_default();
        assert_eq!(
            two.pointer("/properties/x/$ref"),
            Some(&json!("#/components/schemas/TwoKind"))
        );
        assert_eq!(
            c.schemas().get("TwoKind"),
            Some(&json!({"type": "string", "enum": ["b"]}))
        );
        let untitled = json!({"type": "object"});
        assert_eq!(
            c.add_root(&untitled, "Fallback", "X"),
            json!({"$ref": "#/components/schemas/Fallback"})
        );
        assert_eq!(pascal("pages.create_draft"), "PagesCreateDraft");
    }

    #[test]
    fn a_recursive_root_refers_to_its_own_component() {
        let mut c = Components::default();
        let task = json!({"title": "TaskView", "type": "object",
        "properties": {
            "subtasks": {"type": "array", "items": {"$ref": "#"}}
        }});
        assert_eq!(
            c.add_root(&task, "F", "Tasks"),
            json!({"$ref": "#/components/schemas/TaskView"})
        );
        assert_eq!(
            c.schemas()
                .get("TaskView")
                .and_then(|t| t.pointer("/properties/subtasks/items/$ref")),
            Some(&json!("#/components/schemas/TaskView"))
        );
    }
}
