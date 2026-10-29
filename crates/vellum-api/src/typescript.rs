// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The TypeScript client types, generated from the OpenAPI document.
//!
//! [`client`] turns every component schema into a TypeScript type and
//! every operation into an entry of an `Operations` map (method, path,
//! input and output), plus an `OPERATIONS` table the hand-written runtime
//! in `web/src/api/client.ts` reads at run time. The output is stable for
//! a given document, so it can be committed and checked for drift.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Map, Value};

/// The header written at the top of the generated file. The licence tags
/// are split so the REUSE linter does not read them as this file's own.
pub const HEADER: &str = concat!(
    "// SPDX-",
    "FileCopyrightText: 2026 The Vellum contributors\n",
    "// SPDX-",
    "License-Identifier: AGPL-3.0-only\n\n",
    "// Generated from the server's operations by `vellum openapi --typescript`.\n",
    "// Do not edit: run `VELLUM_BLESS=1 cargo test -p vellum-cli typescript_client`.\n",
);

/// The deepest schema nesting rendered; deeper parts become `unknown`.
const MAX_DEPTH: usize = 32;

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A component name as a TypeScript identifier.
fn type_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
        format!("T{cleaned}")
    } else {
        cleaned
    }
}

fn literal(value: &Value) -> String {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => value.to_string(),
        _ => "unknown".to_owned(),
    }
}

fn property_key(name: &str) -> String {
    if is_identifier(name) {
        name.to_owned()
    } else {
        Value::String(name.to_owned()).to_string()
    }
}

/// A `JSDoc` comment for `schema`'s description, indented by `indent`.
fn doc_comment(schema: &Value, indent: &str) -> String {
    let Some(text) = schema
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return String::new();
    };
    let text = text.replace("*/", "*\\/");
    let mut out = format!("{indent}/**\n");
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() {
            let _ = writeln!(out, "{indent} *");
        } else {
            let _ = writeln!(out, "{indent} * {line}");
        }
    }
    let _ = writeln!(out, "{indent} */");
    out
}

fn union(parts: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for part in parts {
        if !seen.contains(&part) {
            seen.push(part);
        }
    }
    match seen.len() {
        0 => "never".to_owned(),
        1 => seen.remove(0),
        _ => seen
            .iter()
            .map(|p| {
                if p.contains(" & ") {
                    format!("({p})")
                } else {
                    p.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn object_type(schema: &Map<String, Value>, depth: usize) -> String {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let properties = schema.get("properties").and_then(Value::as_object);
    let extra = match schema.get("additionalProperties") {
        Some(Value::Bool(false)) => None,
        Some(Value::Object(s)) => Some(ts_type(&Value::Object(s.clone()), depth + 1)),
        // Absent or `true`: other keys may appear.
        _ if properties.is_none_or(Map::is_empty) => Some("unknown".to_owned()),
        _ => None,
    };
    let Some(properties) = properties.filter(|p| !p.is_empty()) else {
        return extra.map_or_else(
            || "Record<string, never>".to_owned(),
            |t| format!("Record<string, {t}>"),
        );
    };
    let indent = "  ".repeat(depth + 1);
    let mut out = String::from("{\n");
    for (name, property) in properties {
        out.push_str(&doc_comment(property, &indent));
        let optional = if required.contains(&name.as_str()) {
            ""
        } else {
            "?"
        };
        let _ = writeln!(
            out,
            "{indent}{}{optional}: {};",
            property_key(name),
            ts_type(property, depth + 1)
        );
    }
    if let Some(extra) = extra {
        let _ = writeln!(out, "{indent}[key: string]: {extra};");
    }
    let _ = write!(out, "{}}}", "  ".repeat(depth));
    out
}

fn single_type(kind: &str, schema: &Map<String, Value>, depth: usize) -> String {
    match kind {
        "string" => "string".to_owned(),
        "integer" | "number" => "number".to_owned(),
        "boolean" => "boolean".to_owned(),
        "null" => "null".to_owned(),
        "array" => {
            if let Some(prefix) = schema.get("prefixItems").and_then(Value::as_array) {
                let items: Vec<String> = prefix.iter().map(|s| ts_type(s, depth + 1)).collect();
                return format!("[{}]", items.join(", "));
            }
            let item = schema
                .get("items")
                .map_or_else(|| "unknown".to_owned(), |s| ts_type(s, depth + 1));
            if item.contains(' ') || item.contains('\n') {
                format!("Array<{item}>")
            } else {
                format!("{item}[]")
            }
        }
        "object" => object_type(schema, depth),
        _ => "unknown".to_owned(),
    }
}

/// The TypeScript type for a JSON Schema.
#[must_use]
pub fn ts_type(schema: &Value, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "unknown".to_owned();
    }
    let Some(schema) = schema.as_object() else {
        return match schema {
            Value::Bool(false) => "never".to_owned(),
            _ => "unknown".to_owned(),
        };
    };
    if let Some(target) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.rsplit('/').next())
    {
        return type_name(target);
    }
    if let Some(value) = schema.get("const") {
        return literal(value);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return union(values.iter().map(literal).collect());
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(options) = schema.get(key).and_then(Value::as_array) {
            let mut parts: Vec<String> = options.iter().map(|o| ts_type(o, depth)).collect();
            // A schema can name a type next to its variants (`type` plus
            // `oneOf` of `required` groups); the variants alone are enough
            // unless they carry no type of their own.
            if parts.iter().all(|p| p == "unknown") {
                let mut base = schema.clone();
                let _ = base.remove(key);
                parts = vec![ts_type(&Value::Object(base), depth)];
            }
            return union(parts);
        }
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        let parts: Vec<String> = all.iter().map(|o| ts_type(o, depth)).collect();
        return match parts.len() {
            0 => "unknown".to_owned(),
            1 => parts.into_iter().next().unwrap_or_default(),
            _ => parts.join(" & "),
        };
    }
    match schema.get("type") {
        Some(Value::String(kind)) => single_type(kind, schema, depth),
        Some(Value::Array(kinds)) => union(
            kinds
                .iter()
                .filter_map(Value::as_str)
                .map(|k| single_type(k, schema, depth))
                .collect(),
        ),
        _ if schema.contains_key("properties") => object_type(schema, depth),
        _ => "unknown".to_owned(),
    }
}

/// One operation, as the generated table describes it.
struct Op {
    id: String,
    method: String,
    path: String,
    mutation: bool,
    input: String,
    output: String,
    summary: String,
}

fn operations(document: &Value) -> Vec<Op> {
    let mut ops = Vec::new();
    let Some(paths) = document.get("paths").and_then(Value::as_object) else {
        return ops;
    };
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else {
            continue;
        };
        for (method, op) in methods {
            let Some(id) = op.get("x-vellum-operation").and_then(Value::as_str) else {
                continue;
            };
            let input = op
                .get("x-vellum-input")
                .map_or_else(|| "Record<string, never>".to_owned(), |s| ts_type(s, 0));
            let output = op
                .pointer("/responses/200/content/application~1json/schema")
                .map_or_else(|| "unknown".to_owned(), |s| ts_type(s, 0));
            ops.push(Op {
                id: id.to_owned(),
                method: method.to_uppercase(),
                path: path.clone(),
                mutation: op.get("x-vellum-kind").and_then(Value::as_str) == Some("mutation"),
                input,
                output,
                summary: op
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
    }
    ops.sort_by(|a, b| a.id.cmp(&b.id));
    ops
}

/// Whether `schema` renders as one object literal, so it can be declared
/// as an `interface` (a union or intersection cannot).
fn is_plain_object(schema: &Value) -> bool {
    let Some(map) = schema.as_object() else {
        return false;
    };
    let combined = ["$ref", "const", "enum", "oneOf", "anyOf", "allOf"]
        .iter()
        .any(|k| map.contains_key(*k));
    let object = match map.get("type") {
        Some(Value::String(t)) => t == "object",
        None => map.contains_key("properties"),
        _ => false,
    };
    !combined && object
}

/// The generated TypeScript module for an OpenAPI document.
#[must_use]
pub fn client(document: &Value) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    let schemas: BTreeMap<String, Value> = document
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    for (name, schema) in &schemas {
        out.push_str(&doc_comment(schema, ""));
        let ts = ts_type(schema, 0);
        if ts.starts_with("{\n") && is_plain_object(schema) {
            let _ = writeln!(out, "export interface {} {ts}\n", type_name(name));
        } else {
            let _ = writeln!(out, "export type {} = {ts};\n", type_name(name));
        }
    }
    let ops = operations(document);
    out.push_str("/** Every operation: its method, path, input and output. */\n");
    out.push_str("export interface Operations {\n");
    for op in &ops {
        if !op.summary.is_empty() {
            let _ = writeln!(out, "  /** {} */", op.summary.replace("*/", "*\\/"));
        }
        let _ = writeln!(
            out,
            "  {}: {{\n    method: {};\n    path: {};\n    input: {};\n    output: {};\n  }};",
            Value::String(op.id.clone()),
            Value::String(op.method.clone()),
            Value::String(op.path.clone()),
            op.input,
            op.output,
        );
    }
    out.push_str("}\n\n");
    out.push_str(
        "/** How to call each operation, for the runtime in `client.ts`. */\n\
         export const OPERATIONS = {\n",
    );
    for op in &ops {
        let _ = writeln!(
            out,
            "  {}: {{ method: {}, path: {}, mutation: {} }},",
            Value::String(op.id.clone()),
            Value::String(op.method.clone()),
            Value::String(op.path.clone()),
            op.mutation,
        );
    }
    out.push_str("} as const;\n");
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn schemas_become_typescript_types() {
        let cases = [
            (json!({"type": "string"}), "string"),
            (json!({"type": ["integer", "null"]}), "number | null"),
            (json!({"enum": ["a", "b"]}), r#""a" | "b""#),
            (json!({"const": 3}), "3"),
            (json!({"$ref": "#/components/schemas/NodeView"}), "NodeView"),
            (
                json!({"anyOf": [{"$ref": "#/components/schemas/A"}, {"type": "null"}]}),
                "A | null",
            ),
            (
                json!({"type": "array", "items": {"type": "string"}}),
                "string[]",
            ),
            (
                json!({"type": "array", "items": {"type": ["string", "null"]}}),
                "Array<string | null>",
            ),
            (
                json!({"type": "object", "additionalProperties": {"type": "number"}}),
                "Record<string, number>",
            ),
            (json!({"type": "object"}), "Record<string, unknown>"),
            (json!({}), "unknown"),
            (json!(true), "unknown"),
        ];
        for (schema, want) in cases {
            assert_eq!(ts_type(&schema, 0), want, "{schema}");
        }
    }

    #[test]
    fn objects_mark_optional_fields_and_quote_odd_keys() {
        let schema = json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string", "description": "The ID."},
                "x-count": {"type": "integer"}
            }
        });
        assert_eq!(
            ts_type(&schema, 0),
            "{\n  /**\n   * The ID.\n   */\n  id: string;\n  \"x-count\"?: number;\n}"
        );
    }

    #[test]
    fn operations_are_listed_with_their_types() {
        let document = json!({
            "paths": {"/v1/drive/items/{id}": {"get": {
                "x-vellum-operation": "drive.get",
                "x-vellum-kind": "query",
                "x-vellum-input": {"$ref": "#/components/schemas/NodeKey"},
                "summary": "Get an item",
                "responses": {"200": {"content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/NodeView"}}}}}
            }}},
            "components": {"schemas": {
                "NodeKey": {"type": "object", "required": ["id"],
                            "properties": {"id": {"type": "string"}}},
                "NodeView": {"type": "object", "properties": {}}
            }}
        });
        let ts = client(&document);
        assert!(
            ts.contains("export interface NodeKey {\n  id: string;\n}"),
            "{ts}"
        );
        assert!(
            ts.contains("\"drive.get\": {\n    method: \"GET\";"),
            "{ts}"
        );
        assert!(
            ts.contains("input: NodeKey;\n    output: NodeView;"),
            "{ts}"
        );
        assert!(ts.contains(
            "\"drive.get\": { method: \"GET\", path: \"/v1/drive/items/{id}\", mutation: false },"
        ));
    }
}
