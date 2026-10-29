// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `GraphQL` schema, generated from the registered operations.
//!
//! Every query operation becomes a field on `Query` and every mutation a
//! field on `Mutation`, named in camel case (`pages.create` →
//! `pagesCreate`) and taking one `input` argument. Types come from the same
//! JSON Schemas as the OpenAPI document: named objects become `GraphQL`
//! object and input types, string enums become enums, 64-bit integers use
//! the `Int64` scalar, and shapes `GraphQL` cannot express (unions, maps)
//! use the `JSON` scalar. Field names are the JSON names, so a `GraphQL`
//! result has the same shape as the REST response.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_graphql::dynamic::{
    Enum, Field, FieldFuture, FieldValue, InputObject, InputValue, Object, ResolverContext, Scalar,
    Schema, SchemaError, TypeRef,
};
use async_graphql::{ErrorExtensions as _, Name};
use serde_json::Value;
use vellum_contracts::{OperationKind, RequestContext};
use vellum_kernel::{Error, RequestId};

use crate::auth::Authenticated;
use crate::openapi::Components;
use crate::registry::{RegisteredOp, Registry, graphql_name};

/// The scalar used for values `GraphQL` cannot type.
pub const JSON_SCALAR: &str = "JSON";
/// The scalar used for integers wider than 32 bits.
pub const INT64_SCALAR: &str = "Int64";
/// Maximum query depth.
pub const MAX_DEPTH: usize = 16;
/// Maximum query complexity.
pub const MAX_COMPLEXITY: usize = 2_000;

/// Per-request data the resolvers need.
#[derive(Debug, Clone)]
pub struct GraphqlRequest {
    /// The request ID.
    pub request_id: RequestId,
    /// The caller.
    pub authenticated: Authenticated,
    /// The incoming `traceparent`, if any.
    pub traceparent: Option<String>,
}

/// How to turn a JSON value into a `GraphQL` field value.
#[derive(Debug, Clone)]
enum Shape {
    Scalar,
    Enum,
    Object,
    List(Box<Self>),
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("__")
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
    {
        cleaned
    } else {
        format!("T{cleaned}")
    }
}

fn pascal(text: &str) -> String {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Separates `null` from a schema: `(inner, nullable)`.
fn split_null(schema: &Value) -> (Value, bool) {
    if let Some(Value::Array(types)) = schema.get("type") {
        let rest: Vec<Value> = types
            .iter()
            .filter(|t| t.as_str() != Some("null"))
            .cloned()
            .collect();
        if rest.len() < types.len() {
            let mut inner = schema.clone();
            if let Some(map) = inner.as_object_mut() {
                let ty = if rest.len() == 1 {
                    rest.into_iter().next().unwrap_or(Value::Null)
                } else {
                    Value::Array(rest)
                };
                let _ = map.insert("type".into(), ty);
            }
            return (inner, true);
        }
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(options)) = schema.get(key) {
            let rest: Vec<&Value> = options
                .iter()
                .filter(|o| o.get("type").and_then(Value::as_str) != Some("null"))
                .collect();
            if rest.len() == 1 && options.len() == 2 {
                return (rest.first().map_or(Value::Null, |v| (*v).clone()), true);
            }
        }
    }
    (schema.clone(), false)
}

fn enum_values(schema: &Value) -> Option<Vec<String>> {
    if let Some(Value::Array(values)) = schema.get("enum") {
        let names: Vec<String> = values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        return (names.len() == values.len()
            && !names.is_empty()
            && names.iter().all(|n| valid_name(n)))
        .then_some(names);
    }
    if let Some(Value::Array(options)) = schema.get("oneOf") {
        let names: Vec<String> = options
            .iter()
            .filter_map(|o| o.get("const").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        return (names.len() == options.len()
            && !names.is_empty()
            && names.iter().all(|n| valid_name(n)))
        .then_some(names);
    }
    None
}

fn wrap(ty: TypeRef, nullable: bool) -> TypeRef {
    if nullable {
        ty
    } else {
        TypeRef::NonNull(Box::new(ty))
    }
}

struct Builder<'a> {
    components: &'a BTreeMap<String, Value>,
    types: Vec<async_graphql::dynamic::Type>,
    outputs: HashSet<String>,
    inputs: HashSet<String>,
    enums: HashSet<String>,
}

impl Builder<'_> {
    fn component<'s>(&'s self, schema: &'s Value) -> Option<(String, &'s Value)> {
        let name = schema
            .get("$ref")?
            .as_str()?
            .strip_prefix("#/components/schemas/")?;
        self.components.get(name).map(|s| (name.to_owned(), s))
    }

    fn scalar(schema: &Value) -> Option<&'static str> {
        match schema.get("type").and_then(Value::as_str)? {
            "string" => Some(TypeRef::STRING),
            "boolean" => Some(TypeRef::BOOLEAN),
            "number" => Some(TypeRef::FLOAT),
            "integer" => match schema.get("format").and_then(Value::as_str) {
                Some("int8" | "int16" | "int32" | "uint8" | "uint16") => Some(TypeRef::INT),
                _ => Some(INT64_SCALAR),
            },
            _ => None,
        }
    }

    fn enum_type(&mut self, name: &str, values: &[String]) -> TypeRef {
        let name = sanitize(name);
        if self.enums.insert(name.clone()) {
            self.types.push(
                Enum::new(name.clone())
                    .items(values.iter().map(String::as_str))
                    .into(),
            );
        }
        TypeRef::named(name)
    }

    /// The output type for a schema, and how to convert values to it.
    fn output(&mut self, schema: &Value, hint: &str) -> (TypeRef, Shape) {
        let (schema, nullable) = split_null(schema);
        let (ty, shape) = self.output_inner(&schema, hint);
        (wrap(ty, nullable), shape)
    }

    fn output_inner(&mut self, schema: &Value, hint: &str) -> (TypeRef, Shape) {
        let (name, schema) = match self.component(schema) {
            Some((name, resolved)) => (name, split_null(resolved).0),
            None => (hint.to_owned(), schema.clone()),
        };
        if let Some(values) = enum_values(&schema) {
            return (self.enum_type(&name, &values), Shape::Enum);
        }
        if let Some(scalar) = Self::scalar(&schema)
            .filter(|_| schema.get("type").and_then(Value::as_str) != Some("object"))
        {
            return (TypeRef::named(scalar), Shape::Scalar);
        }
        if schema.get("type").and_then(Value::as_str) == Some("array") {
            let items = schema.get("items").cloned().unwrap_or(Value::Bool(true));
            let (inner, shape) = self.output(&items, &format!("{name}Item"));
            return (TypeRef::List(Box::new(inner)), Shape::List(Box::new(shape)));
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .filter(|p| !p.is_empty() && p.keys().all(|k| valid_name(k)));
        let Some(properties) = properties.cloned() else {
            return (TypeRef::named(JSON_SCALAR), Shape::Scalar);
        };
        let type_name = sanitize(&name);
        if self.outputs.insert(type_name.clone()) {
            let required = required_set(&schema);
            let mut object = Object::new(type_name.clone());
            if let Some(description) = schema.get("description").and_then(Value::as_str) {
                object = object.description(description);
            }
            for (field, field_schema) in &properties {
                let (ty, shape) =
                    self.output(field_schema, &format!("{type_name}{}", pascal(field)));
                let ty = if required.contains(field.as_str()) {
                    ty
                } else {
                    nullable(ty)
                };
                let key = field.clone();
                let mut f = Field::new(field.clone(), ty, move |ctx: ResolverContext<'_>| {
                    let value = ctx
                        .parent_value
                        .downcast_ref::<Value>()
                        .and_then(|parent| parent.get(&key))
                        .cloned()
                        .unwrap_or(Value::Null);
                    FieldFuture::Value(to_field_value(value, &shape))
                });
                if let Some(description) = field_schema.get("description").and_then(Value::as_str) {
                    f = f.description(description);
                }
                object = object.field(f);
            }
            self.types.push(object.into());
        }
        (TypeRef::named(type_name), Shape::Object)
    }

    /// The input type for a schema.
    fn input(&mut self, schema: &Value, hint: &str) -> TypeRef {
        let (schema, nullable) = split_null(schema);
        let ty = self.input_inner(&schema, hint);
        wrap(ty, nullable)
    }

    fn input_inner(&mut self, schema: &Value, hint: &str) -> TypeRef {
        let (name, schema) = match self.component(schema) {
            Some((name, resolved)) => (name, split_null(resolved).0),
            None => (hint.to_owned(), schema.clone()),
        };
        if let Some(values) = enum_values(&schema) {
            return self.enum_type(&name, &values);
        }
        if let Some(scalar) = Self::scalar(&schema)
            .filter(|_| schema.get("type").and_then(Value::as_str) != Some("object"))
        {
            return TypeRef::named(scalar);
        }
        if schema.get("type").and_then(Value::as_str) == Some("array") {
            let items = schema.get("items").cloned().unwrap_or(Value::Bool(true));
            return TypeRef::List(Box::new(self.input(&items, &format!("{name}Item"))));
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .filter(|p| !p.is_empty() && p.keys().all(|k| valid_name(k)));
        let Some(properties) = properties.cloned() else {
            return TypeRef::named(JSON_SCALAR);
        };
        let base = sanitize(&name);
        let type_name = if base.ends_with("Input") {
            base
        } else {
            format!("{base}Input")
        };
        if self.inputs.insert(type_name.clone()) {
            let required = required_set(&schema);
            let mut object = InputObject::new(type_name.clone());
            if let Some(description) = schema.get("description").and_then(Value::as_str) {
                object = object.description(description);
            }
            for (field, field_schema) in &properties {
                let ty = self.input(field_schema, &format!("{type_name}{}", pascal(field)));
                let ty = if required.contains(field.as_str()) {
                    ty
                } else {
                    nullable(ty)
                };
                let mut v = InputValue::new(field.clone(), ty);
                if let Some(description) = field_schema.get("description").and_then(Value::as_str) {
                    v = v.description(description);
                }
                object = object.field(v);
            }
            self.types.push(object.into());
        }
        TypeRef::named(type_name)
    }
}

fn required_set(schema: &Value) -> HashSet<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn nullable(ty: TypeRef) -> TypeRef {
    match ty {
        TypeRef::NonNull(inner) => *inner,
        other => other,
    }
}

fn to_field_value(value: Value, shape: &Shape) -> Option<FieldValue<'static>> {
    if value.is_null() {
        return None;
    }
    match shape {
        Shape::Scalar => Some(FieldValue::value(
            async_graphql::Value::from_json(value).unwrap_or_default(),
        )),
        Shape::Enum => value
            .as_str()
            .map(|s| FieldValue::value(async_graphql::Value::Enum(Name::new(s)))),
        Shape::Object => Some(FieldValue::owned_any(value)),
        Shape::List(item) => match value {
            Value::Array(items) => {
                Some(FieldValue::list(items.into_iter().map(|v| {
                    to_field_value(v, item).unwrap_or(FieldValue::NULL)
                })))
            }
            _ => None,
        },
    }
}

fn graphql_error(error: &Error, request_id: RequestId) -> async_graphql::Error {
    let body = vellum_contracts::ApiError::from_error(error, request_id);
    async_graphql::Error::new(body.message.clone()).extend_with(|_, e| {
        e.set("code", body.code.clone());
        e.set("retryable", body.retryable);
        e.set("request_id", body.request_id.clone());
    })
}

fn operation_field(
    builder: &mut Builder<'_>,
    components: &Components,
    registered: &Arc<RegisteredOp>,
    input: &Value,
    output: &Value,
) -> Field {
    let op = &registered.op;
    let name = graphql_name(op.id());
    let owner = pascal(&name);
    let (out_ty, shape) = builder.output(output, &format!("{owner}Output"));
    let resolved_input = components.resolve(input);
    let has_input = resolved_input
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty());
    let input_ty = has_input.then(|| builder.input(input, &format!("{owner}Input")));
    let registered = Arc::clone(registered);
    let mut field = Field::new(name, out_ty, move |ctx: ResolverContext<'_>| {
        let registered = Arc::clone(&registered);
        let shape = shape.clone();
        FieldFuture::new(async move {
            let request = ctx.data::<GraphqlRequest>()?;
            let input = match ctx.args.get("input") {
                Some(value) => value
                    .as_value()
                    .clone()
                    .into_json()
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?,
                None => Value::Object(serde_json::Map::new()),
            };
            let context = RequestContext {
                request_id: request.request_id,
                principal: request.authenticated.principal.clone(),
                deadline: Instant::now() + registered.op.time_budget(),
                traceparent: request.traceparent.clone(),
            };
            let output = registered
                .invoke(context, input)
                .await
                .map_err(|e| graphql_error(&e, request.request_id))?;
            Ok(to_field_value(output, &shape))
        })
    });
    if let Some(ty) = input_ty {
        field = field.argument(InputValue::new("input", ty));
    }
    let description = [op.summary_text(), op.description_text()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !description.is_empty() {
        field = field.description(description);
    }
    if op.is_deprecated() {
        field = field.deprecation(Some("deprecated; see the API changelog"));
    }
    field
}

/// Builds the executable schema.
///
/// # Errors
///
/// A [`SchemaError`] if the generated types are inconsistent (a bug).
pub fn schema(registry: &Registry, version: &'static str) -> Result<Schema, SchemaError> {
    let mut components = Components::default();
    let ops: Vec<(Arc<RegisteredOp>, Value, Value)> = registry
        .operations()
        .iter()
        .map(|r| {
            let owner = pascal(&graphql_name(r.op.id()));
            let input = components.add_root(
                &Value::from(r.op.input_schema().clone()),
                &format!("{owner}Input"),
                &owner,
            );
            let output = components.add_root(
                &Value::from(r.op.output_schema().clone()),
                &format!("{owner}Output"),
                &owner,
            );
            (Arc::clone(r), input, output)
        })
        .collect();
    let mut builder = Builder {
        components: components.schemas(),
        types: Vec::new(),
        outputs: HashSet::new(),
        inputs: HashSet::new(),
        enums: HashSet::new(),
    };
    let mut query = Object::new("Query").field(Field::new(
        "apiVersion",
        TypeRef::named_nn(TypeRef::STRING),
        move |_| FieldFuture::Value(Some(FieldValue::value(version))),
    ));
    let mut mutation = Object::new("Mutation");
    let mut has_mutation = false;
    for (registered, input, output) in &ops {
        let field = operation_field(&mut builder, &components, registered, input, output);
        match registered.op.kind() {
            OperationKind::Query => query = query.field(field),
            OperationKind::Mutation => {
                mutation = mutation.field(field);
                has_mutation = true;
            }
        }
    }
    let mut schema = Schema::build("Query", has_mutation.then_some("Mutation"), None)
        .register(query)
        .register(Scalar::new(JSON_SCALAR).description("Any JSON value."))
        .register(
            Scalar::new(INT64_SCALAR)
                .description("A signed or unsigned integer wider than 32 bits, as a JSON number.")
                .validator(
                    |v| matches!(v, async_graphql::Value::Number(n) if n.is_i64() || n.is_u64()),
                ),
        )
        .limit_depth(MAX_DEPTH)
        .limit_complexity(MAX_COMPLEXITY);
    if has_mutation {
        schema = schema.register(mutation);
    }
    for ty in builder.types {
        schema = schema.register(ty);
    }
    schema.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn helpers() {
        assert!(valid_name("pagesCreate"));
        assert!(!valid_name("write:draft"));
        assert!(!valid_name("__x"));
        assert_eq!(sanitize("Vec<Page>"), "Vec_Page_");
        assert_eq!(sanitize("1x"), "T1x");
        assert_eq!(pascal("space_id"), "SpaceId");
        let (inner, n) = split_null(&json!({"type": ["string", "null"]}));
        assert!(n);
        assert_eq!(inner, json!({"type": "string"}));
        let (inner, n) = split_null(&json!({"anyOf": [{"$ref": "#/x"}, {"type": "null"}]}));
        assert!(n);
        assert_eq!(inner, json!({"$ref": "#/x"}));
        assert_eq!(
            enum_values(&json!({"enum": ["a", "b"]})),
            Some(vec!["a".into(), "b".into()])
        );
        assert_eq!(enum_values(&json!({"enum": ["write:draft"]})), None);
        assert_eq!(
            enum_values(&json!({"oneOf": [{"const": "x"}, {"const": "y"}]})).map(|v| v.len()),
            Some(2)
        );
    }
}
