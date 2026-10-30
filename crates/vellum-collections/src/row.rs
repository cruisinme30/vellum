// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Row pages: `rows/<slug>.md`.
//!
//! ```markdown
//! ---
//! id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3G
//! t: R1 · Replay 7 days
//! pr: P0
//! est: 8
//! rel: [01JBQ8Y4ZK6V3W2N5R7T9X1C3H]
//! ---
//! Notes about this requirement.
//! ```
//!
//! Stored properties live in front matter under their keys. Computed
//! properties (formula, rollup, created) are never written: they are derived
//! on every save, so files cannot drift from their inputs and external
//! edits never conflict on derived data. Unknown keys and values that do not
//! fit their type are kept verbatim and written back unchanged.

use std::collections::BTreeMap;

use crate::yaml::{Mapping, Yaml};
use vellum_kernel::DocId;

use crate::error::Diagnostic;
use crate::schema::{PropertyType, Schema};
use crate::value::{DateValue, Value};

/// Longest file-name slug, in characters, before `.md`.
pub const MAX_SLUG_CHARS: usize = 60;

/// One row: a page whose front matter carries the property values.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The row page's identity.
    pub id: DocId,
    values: BTreeMap<String, Value>,
    extra: Mapping,
    /// The page body (Vellum Markdown), after the front matter.
    pub body: String,
}

impl Row {
    /// An empty row.
    #[must_use]
    pub const fn new(id: DocId) -> Self {
        Self {
            id,
            values: BTreeMap::new(),
            extra: Mapping::new(),
            body: String::new(),
        }
    }

    /// The stored value of a property key ([`Value::Empty`] when unset).
    #[must_use]
    pub fn get(&self, key: &str) -> &Value {
        static EMPTY: Value = Value::Empty;
        self.values.get(key).unwrap_or(&EMPTY)
    }

    /// Stored values by key.
    #[must_use]
    pub const fn values(&self) -> &BTreeMap<String, Value> {
        &self.values
    }

    /// Sets a stored value without checking it; [`crate::Workspace`] checks
    /// edits against the schema first.
    pub(crate) fn set_raw(&mut self, key: &str, value: Value) {
        let _ = self.extra.remove(key);
        if value.is_empty() {
            let _ = self.values.remove(key);
        } else {
            let _ = self.values.insert(key.to_owned(), value);
        }
    }

    /// The related row IDs stored under a relation key.
    #[must_use]
    pub fn links(&self, key: &str) -> Vec<DocId> {
        self.get(key)
            .items()
            .iter()
            .filter_map(|v| v.as_text().and_then(|t| t.parse().ok()))
            .collect()
    }

    /// The title text.
    #[must_use]
    pub fn title(&self, schema: &Schema) -> String {
        schema
            .title_property()
            .map(|p| self.get(&p.key).to_string())
            .unwrap_or_default()
    }

    /// The file name for this row: the title slug plus `.md`.
    #[must_use]
    pub fn file_name(&self, schema: &Schema) -> String {
        format!("{}.md", slug(&self.title(schema)))
    }

    /// Front-matter keys that are not properties, such as `icon` and
    /// `cover`, in file order.
    #[must_use]
    pub const fn extra(&self) -> &Mapping {
        &self.extra
    }

    /// Parses a row page. Never fails: problems become diagnostics, bad
    /// values are preserved verbatim, and a page without an `id` gets a new
    /// one (with a warning, so the caller writes it back).
    #[must_use]
    pub fn parse(
        text: &str,
        schema: &Schema,
        location: &str,
    ) -> (Self, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let (front, body) = split_front_matter(text);
        let mapping = match front.map(crate::yaml::from_str::<Yaml>) {
            Some(Ok(Yaml::Map(m))) => m,
            None | Some(Ok(Yaml::Null)) => Mapping::new(),
            Some(Ok(_)) => {
                diags.push(Diagnostic::error(
                    location,
                    "front matter is not a mapping",
                ));
                Mapping::new()
            }
            Some(Err(e)) => {
                diags.push(Diagnostic::error(
                    location,
                    format!("front matter: {e}"),
                ));
                Mapping::new()
            }
        };
        let parsed = mapping
            .get("id")
            .and_then(Yaml::as_str)
            .and_then(|s| s.parse::<DocId>().ok());
        let id = parsed.unwrap_or_else(|| {
            let id = DocId::generate();
            diags.push(Diagnostic::warning(
                location,
                format!("row has no valid id; assigned {id}"),
            ));
            id
        });
        let mut row = Self {
            id,
            values: BTreeMap::new(),
            extra: Mapping::new(),
            body: body.to_owned(),
        };
        for (key, v) in mapping {
            if key == "id" {
                continue;
            }
            match schema.property(&key) {
                Some(p) if p.kind.is_stored() => match from_yaml(&p.kind, &v) {
                    Ok(value) => {
                        if !value.is_empty() {
                            let _ = row.values.insert(key, value);
                        }
                    }
                    Err(why) => {
                        diags.push(Diagnostic::error(
                            location,
                            format!(
                                "{} ({key}): {why}; kept as written",
                                p.name
                            ),
                        ));
                        row.extra.insert(key, v);
                    }
                },
                _ => {
                    row.extra.insert(key, v);
                }
            }
        }
        (row, diags)
    }

    /// Writes the canonical page: `id`, then stored properties in schema
    /// order, then other keys in their original order, then the body.
    #[must_use]
    pub fn to_markdown(&self, schema: &Schema) -> String {
        let mut m = Mapping::new();
        m.insert("id", Yaml::String(self.id.to_string()));
        for p in &schema.properties {
            if let Some(v) = self.values.get(&p.key) {
                m.insert(p.key.clone(), to_yaml(&p.kind, v));
            }
        }
        for (k, v) in self.extra.iter() {
            if !m.contains_key(k) {
                m.insert(k.clone(), v.clone());
            }
        }
        let yaml = crate::yaml::to_string(&Yaml::Map(m)).unwrap_or_default();
        format!("---\n{yaml}---\n{}", self.body)
    }
}

/// Splits `---` front matter from the body. Returns `(None, text)` when
/// the page has none.
fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            let front = rest.get(..offset).unwrap_or_default();
            let body = rest.get(offset + line.len()..).unwrap_or_default();
            return (Some(front), body);
        }
        offset += line.len();
    }
    (None, text)
}

/// Converts a front-matter value into a typed [`Value`].
///
/// # Errors
///
/// Returns why the value does not fit the property type.
pub(crate) fn from_yaml(
    kind: &PropertyType,
    v: &Yaml,
) -> Result<Value, String> {
    if v.is_null() {
        return Ok(Value::Empty);
    }
    let scalar_text = |v: &Yaml| -> Option<String> {
        match v {
            Yaml::String(s) => Some(s.clone()),
            Yaml::Int(n) => Some(n.to_string()),
            Yaml::Float(n) => Some(crate::value::format_number(*n)),
            Yaml::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    };
    let list = |v: &Yaml| -> Option<Vec<String>> {
        match v {
            Yaml::Seq(items) => items.iter().map(scalar_text).collect(),
            other => scalar_text(other).map(|s| vec![s]),
        }
    };
    match kind {
        PropertyType::Title
        | PropertyType::Text
        | PropertyType::Url
        | PropertyType::Select { .. } => scalar_text(v)
            .map(Value::Text)
            .ok_or_else(|| "expected text".into()),
        PropertyType::Number { .. } => match v {
            #[allow(clippy::cast_precision_loss)]
            Yaml::Int(n) => Some(Value::Number(*n as f64)),
            Yaml::Float(n) => {
                Some(*n).filter(|f| f.is_finite()).map(Value::Number)
            }
            Yaml::String(s) => parse_number(s).map(Value::Number),
            _ => None,
        }
        .ok_or_else(|| "expected a number".into()),
        PropertyType::Checkbox => match v {
            Yaml::Bool(b) => Some(*b),
            Yaml::String(s) => parse_bool(s),
            _ => None,
        }
        .map(Value::Bool)
        .ok_or_else(|| "expected true or false".into()),
        PropertyType::Date => v
            .as_str()
            .and_then(DateValue::parse)
            .map(Value::Date)
            .ok_or_else(|| "expected a date like 2026-11-24".into()),
        PropertyType::Multi { .. } => list(v)
            .map(|items| {
                Value::List(items.into_iter().map(Value::Text).collect())
            })
            .ok_or_else(|| "expected a list of options".into()),
        PropertyType::Person => match list(v) {
            Some(mut items) if items.len() == 1 => {
                Ok(Value::Text(items.remove(0)))
            }
            Some(items) => {
                Ok(Value::List(items.into_iter().map(Value::Text).collect()))
            }
            None => Err("expected a person handle or a list of them".into()),
        },
        PropertyType::Relation { .. } => {
            let items = list(v).ok_or("expected a list of row ids")?;
            let mut ids = Vec::with_capacity(items.len());
            for item in items {
                let id: DocId = item.parse().map_err(|e| format!("{e}"))?;
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            Ok(relation_value(&ids))
        }
        PropertyType::Rollup { .. }
        | PropertyType::Formula { .. }
        | PropertyType::Created => Ok(Value::Empty),
    }
}

/// The stored form of a relation: a list of row IDs as text.
pub(crate) fn relation_value(ids: &[DocId]) -> Value {
    Value::List(ids.iter().map(|id| Value::Text(id.to_string())).collect())
}

pub(crate) fn parse_number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|f| f.is_finite())
}

pub(crate) fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "1" | "x" | "✓" => Some(true),
        "false" | "no" | "n" | "0" | "" => Some(false),
        _ => None,
    }
}

fn to_yaml(kind: &PropertyType, v: &Value) -> Yaml {
    let always_list = matches!(
        kind,
        PropertyType::Multi { .. } | PropertyType::Relation { .. }
    );
    match v {
        Value::List(items) => {
            Yaml::Seq(items.iter().map(scalar_yaml).collect())
        }
        other if always_list => Yaml::Seq(vec![scalar_yaml(other)]),
        other => scalar_yaml(other),
    }
}

fn scalar_yaml(v: &Value) -> Yaml {
    match v {
        Value::Empty => Yaml::Null,
        Value::Text(t) => Yaml::String(t.clone()),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => {
            // Integral and inside the exact range of f64, so the cast is exact.
            #[allow(clippy::cast_possible_truncation)]
            let i = *n as i64;
            Yaml::Int(i)
        }
        Value::Number(n) => Yaml::Float(*n),
        Value::Bool(b) => Yaml::Bool(*b),
        Value::Date(d) => Yaml::String(d.to_string()),
        Value::List(items) => {
            Yaml::Seq(items.iter().map(scalar_yaml).collect())
        }
    }
}

/// Turns a title into a file-name slug: lowercase letters and digits
/// joined by single hyphens, at most [`MAX_SLUG_CHARS`] characters.
#[must_use]
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
            if out.chars().count() >= MAX_SLUG_CHARS {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::TEST_SCHEMA;

    const ROW: &str = "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C3G\n\
        t: R1 · Replay 7 days\npr: P0\nest: 8\nicon: 🎯\n---\n\
        Notes about this requirement.\n";

    #[test]
    fn parses_and_round_trips() {
        let schema = Schema::from_yaml(TEST_SCHEMA).unwrap();
        let (row, diags) = Row::parse(ROW, &schema, "rows/r1.md");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(row.get("t"), &Value::text("R1 · Replay 7 days"));
        assert_eq!(row.get("est"), &Value::Number(8.0));
        assert_eq!(row.body, "Notes about this requirement.\n");
        assert_eq!(row.file_name(&schema), "r1-replay-7-days.md");
        assert_eq!(row.to_markdown(&schema), ROW);
    }

    #[test]
    fn bad_values_are_kept_with_a_diagnostic() {
        let schema = Schema::from_yaml(TEST_SCHEMA).unwrap();
        let text =
            "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C3G\nt: R2\nest: lots\n---\n";
        let (row, diags) = Row::parse(text, &schema, "rows/r2.md");
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("Estimate (est): expected a number")
        );
        assert_eq!(row.get("est"), &Value::Empty);
        assert!(row.to_markdown(&schema).contains("est: lots"));
    }

    #[test]
    fn missing_id_gets_one() {
        let schema = Schema::from_yaml(TEST_SCHEMA).unwrap();
        let (_, diags) = Row::parse("no front matter", &schema, "rows/x.md");
        assert!(diags[0].message.contains("assigned"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("R1 · Replay 7 days"), "r1-replay-7-days");
        assert_eq!(slug("  ¿Qué?  "), "qué");
        assert_eq!(slug("!!!"), "untitled");
        assert_eq!(slug(&"a".repeat(100)).len(), MAX_SLUG_CHARS);
    }
}
