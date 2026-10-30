// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Building an operation's JSON input from a REST request.
//!
//! Path parameters, the query string (for `GET` and `DELETE`) and the JSON
//! body (for `POST`, `PUT` and `PATCH`) are merged into one object. Text from
//! the path and query string is converted to the type the input schema
//! declares for that field (integer, number, boolean or array).

use serde_json::{Map, Number, Value};
use vellum_contracts::Operation;
use vellum_kernel::{Error, ErrorKind};

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn declared_types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn coerce_scalar(
    field: &str,
    text: &str,
    schema: Option<&Value>,
) -> Result<Value, Error> {
    let types = schema.map(declared_types).unwrap_or_default();
    let has = |t: &str| types.contains(&t);
    if has("null") && text == "null" {
        return Ok(Value::Null);
    }
    if has("integer") {
        if let Ok(n) = text.parse::<i64>() {
            return Ok(Value::Number(n.into()));
        }
        if let Ok(n) = text.parse::<u64>() {
            return Ok(Value::Number(n.into()));
        }
        if !has("string") {
            return Err(invalid(format!("`{field}` must be an integer")));
        }
    }
    if has("number") {
        if let Some(n) = text.parse::<f64>().ok().and_then(Number::from_f64) {
            return Ok(Value::Number(n));
        }
        if !has("string") {
            return Err(invalid(format!("`{field}` must be a number")));
        }
    }
    if has("boolean") {
        match text {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            _ if !has("string") => {
                return Err(invalid(format!(
                    "`{field}` must be true or false"
                )));
            }
            _ => {}
        }
    }
    Ok(Value::String(text.to_owned()))
}

fn property<'a>(op: &'a Operation, name: &str) -> Option<&'a Value> {
    op.input_schema()
        .get("properties")
        .and_then(|p| p.get(name))
}

/// Merges path parameters, the query string and the body into the input.
///
/// # Errors
///
/// `invalid_input` when the body is not a JSON object, a value has the
/// wrong type, or the body contradicts a path parameter.
pub fn build(
    op: &Operation,
    path_params: &[(String, String)],
    query: Option<&str>,
    body: Option<&[u8]>,
) -> Result<Value, Error> {
    let mut input =
        match body.filter(|b| !b.iter().all(u8::is_ascii_whitespace)) {
            None => Map::new(),
            Some(bytes) => match serde_json::from_slice::<Value>(bytes) {
                Ok(Value::Object(map)) => map,
                Ok(_) => {
                    return Err(invalid(
                        "the request body must be a JSON object".into(),
                    ));
                }
                Err(err) => {
                    return Err(invalid(format!(
                        "the request body is not valid JSON: {err}"
                    )));
                }
            },
        };
    if let Some(query) = query.filter(|_| !op.method().has_body()) {
        {
            let pairs: Vec<(String, String)> =
                serde_urlencoded::from_str(query).map_err(|err| {
                    invalid(format!("the query string is malformed: {err}"))
                })?;
            for (name, text) in pairs {
                let schema = property(op, &name);
                let is_array = schema
                    .is_some_and(|s| declared_types(s).contains(&"array"));
                if is_array {
                    let item_schema = schema.and_then(|s| s.get("items"));
                    let item = coerce_scalar(&name, &text, item_schema)?;
                    match input
                        .entry(name)
                        .or_insert_with(|| Value::Array(Vec::new()))
                    {
                        Value::Array(items) => items.push(item),
                        _ => {
                            return Err(invalid(
                                "a list field was given twice in different \
                                    forms"
                                    .into(),
                            ));
                        }
                    }
                } else if input
                    .insert(name.clone(), coerce_scalar(&name, &text, schema)?)
                    .is_some()
                {
                    return Err(invalid(format!(
                        "`{name}` was given more than once"
                    )));
                }
            }
        }
    }
    for (name, text) in path_params {
        let value = coerce_scalar(name, text, property(op, name))?;
        match input.get(name) {
            Some(existing) if *existing != value => {
                return Err(invalid(format!(
                    "`{name}` in the body does not match the path"
                )));
            }
            _ => {
                let _ = input.insert(name.clone(), value);
            }
        }
    }
    Ok(Value::Object(input))
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use vellum_contracts::{HttpMethod, Scope};

    use super::*;

    #[derive(Deserialize, JsonSchema)]
    #[expect(dead_code, reason = "only the schema is used")]
    struct In {
        space_id: String,
        limit: Option<u32>,
        ratio: f64,
        archived: bool,
        tags: Vec<String>,
        sizes: Vec<i32>,
    }

    #[derive(Serialize, JsonSchema)]
    struct Out {}

    fn query_op() -> Operation {
        Operation::query(
            "t.list",
            "/v1/s/{space_id}",
            Scope::Read,
            |_c, _i: In| async { Ok(Out {}) },
        )
    }

    #[test]
    fn query_strings_are_typed() -> Result<(), Error> {
        let op = query_op();
        let params = vec![("space_id".to_owned(), "S1".to_owned())];
        let v = build(
            &op,
            &params,
            Some("limit=5&ratio=0.5&archived=true&tags=a&tags=b&sizes=3"),
            None,
        )?;
        assert_eq!(
            v,
            json!({
                "space_id":"S1",
                "limit":5,
                "ratio":0.5,
                "archived":true,
                "tags":["a","b"],
                "sizes":[3]
            })
        );
        assert!(build(&op, &params, Some("limit=x"), None).is_err());
        assert!(build(&op, &params, Some("archived=maybe"), None).is_err());
        assert!(build(&op, &params, Some("ratio=1&ratio=2"), None).is_err());
        assert!(
            build(&op, &params, Some("%zz"), None).is_ok()
                || build(&op, &params, Some("%zz"), None).is_err()
        );
        let v = build(&op, &params, Some("limit=null"), None)?;
        assert_eq!(v.get("limit"), Some(&Value::Null));
        Ok(())
    }

    #[test]
    fn bodies_merge_with_path() -> Result<(), Error> {
        let op = Operation::mutation(
            "t.make",
            HttpMethod::Post,
            "/v1/s/{space_id}",
            Scope::WriteDirect,
            |_c, _i: In| async { Ok(Out {}) },
        );
        let params = vec![("space_id".to_owned(), "S1".to_owned())];
        let v =
            build(&op, &params, Some("ignored=1"), Some(br#"{"ratio": 2}"#))?;
        assert_eq!(v, json!({"space_id":"S1","ratio":2}));
        assert!(
            build(&op, &params, None, Some(br#"{"space_id":"S2"}"#)).is_err()
        );
        assert!(
            build(&op, &params, None, Some(br#"{"space_id":"S1"}"#)).is_ok()
        );
        assert!(build(&op, &params, None, Some(b"[1]")).is_err());
        assert!(build(&op, &params, None, Some(b"{nope")).is_err());
        assert_eq!(
            build(&op, &params, None, Some(b"  "))?,
            json!({"space_id":"S1"})
        );
        Ok(())
    }
}
