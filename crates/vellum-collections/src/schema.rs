// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `schema.yaml`: a collection's properties and saved views.
//!
//! ```yaml
//! id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E
//! title: Requirements tracker
//! properties:
//!   - {key: t, name: Requirement, type: title}
//!   - {key: pr, name: Priority, type: select, options: [P0, P1, P2]}
//!   - {key: est, name: Estimate, type: number}
//!   - {key: rel, name: Release, type: relation, to: 01JBQ8Y4ZK6V3W2N5R7T9X1C3F, back: rq}
//!   - {key: f1, name: Remaining, type: formula, f: 'round(Estimate * 0.5, 1)'}
//! views:
//!   - {name: All, type: table}
//!   - {name: By priority, type: board, group: pr}
//! ```
//!
//! Each property has a short stable `key` (the front-matter field in row
//! files) and a display `name` that may change freely. Parsing validates
//! everything that can be checked inside one collection; checks that span
//! collections (relation pairs, rollup targets) run when the collection
//! joins a [`crate::Workspace`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use vellum_kernel::CollectionId;

use crate::error::SchemaError;
use crate::formula::{self, Formula};
use crate::view::{ViewDef, ViewDefRaw};

/// Front-matter keys every row page reserves; property keys may not use them.
pub const RESERVED_KEYS: [&str; 5] = ["id", "icon", "cover", "vellum", "title"];

/// Longest allowed property key.
pub const MAX_KEY_LEN: usize = 32;

/// Most properties one collection may have.
pub const MAX_PROPERTIES: usize = 256;

/// How rollups combine the related rows' values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    /// Number of related rows.
    Count,
    /// Sum of numeric values.
    Sum,
    /// Mean of numeric values.
    Avg,
    /// Smallest number or earliest date.
    Min,
    /// Largest number or latest date.
    Max,
    /// Number of checked values.
    Checked,
    /// Checked values as a fraction of related rows (0–1).
    Pct,
    /// Number of distinct non-empty values.
    Unique,
    /// The values themselves, as a list.
    Show,
}

impl Aggregation {
    /// Every aggregation, in documentation order.
    pub const ALL: [Self; 9] = [
        Self::Count,
        Self::Sum,
        Self::Avg,
        Self::Min,
        Self::Max,
        Self::Checked,
        Self::Pct,
        Self::Unique,
        Self::Show,
    ];
}

/// How a numeric result is displayed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NumberFormat {
    /// `1234.5`
    #[default]
    Plain,
    /// `76%` for `0.76`.
    Percent,
}

/// One select or multi-select option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SelectOption {
    /// The option text, as stored in row files.
    pub name: String,
    /// A palette colour name for chips, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// The type of a property, with its type-specific settings.
#[derive(Debug, Clone)]
pub enum PropertyType {
    /// The row's title; exactly one per collection.
    Title,
    /// Free text.
    Text,
    /// A number.
    Number {
        /// Display format.
        format: NumberFormat,
    },
    /// One option from a list.
    Select {
        /// The options, in board and sort order.
        options: Vec<SelectOption>,
    },
    /// Any number of options from a list.
    Multi {
        /// The options, in display order.
        options: Vec<SelectOption>,
    },
    /// One or more people, by handle.
    Person,
    /// A date, optionally with a time.
    Date,
    /// True or false.
    Checkbox,
    /// A web address.
    Url,
    /// Links to rows of a collection, kept two-way.
    Relation {
        /// The collection the linked rows belong to.
        to: CollectionId,
        /// The paired relation property in that collection, if two-way.
        back: Option<String>,
    },
    /// An aggregate over the rows a relation links to.
    Rollup {
        /// The relation property (in this collection) to follow.
        via: String,
        /// The property to read on the related rows.
        prop: String,
        /// How to combine the values.
        agg: Aggregation,
        /// Display format.
        format: NumberFormat,
    },
    /// A computed expression.
    Formula {
        /// The compiled formula.
        formula: Formula,
        /// Display format.
        format: NumberFormat,
    },
    /// When the row was created, read from its ULID.
    Created,
}

impl PropertyType {
    /// The type's name as written in `schema.yaml`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Text => "text",
            Self::Number { .. } => "number",
            Self::Select { .. } => "select",
            Self::Multi { .. } => "multi",
            Self::Person => "person",
            Self::Date => "date",
            Self::Checkbox => "checkbox",
            Self::Url => "url",
            Self::Relation { .. } => "relation",
            Self::Rollup { .. } => "rollup",
            Self::Formula { .. } => "formula",
            Self::Created => "created",
        }
    }

    /// Whether the value is written in row files (not computed).
    #[must_use]
    pub const fn is_stored(&self) -> bool {
        !matches!(
            self,
            Self::Rollup { .. } | Self::Formula { .. } | Self::Created
        )
    }

    /// The options of a select or multi-select.
    #[must_use]
    pub fn options(&self) -> &[SelectOption] {
        match self {
            Self::Select { options } | Self::Multi { options } => options,
            _ => &[],
        }
    }
}

/// One column of a collection.
#[derive(Debug, Clone)]
pub struct Property {
    /// Stable short key used in row front matter.
    pub key: String,
    /// Display name, used by formulas and the UI.
    pub name: String,
    /// The type and its settings.
    pub kind: PropertyType,
}

/// A parsed, validated `schema.yaml`.
#[derive(Debug, Clone)]
pub struct Schema {
    /// The collection's identity.
    pub id: CollectionId,
    /// The collection's title.
    pub title: String,
    /// The properties, in column order.
    pub properties: Vec<Property>,
    /// Saved views, in tab order.
    pub views: Vec<ViewDef>,
}

impl Schema {
    /// Parses and validates `schema.yaml`.
    ///
    /// # Errors
    ///
    /// Returns a [`SchemaError`] naming the first problem found.
    pub fn from_yaml(text: &str) -> Result<Self, SchemaError> {
        let raw: SchemaRaw =
            serde_yaml_ng::from_str(text).map_err(|e| SchemaError::Yaml(e.to_string()))?;
        Self::from_raw(raw)
    }

    /// Writes the canonical `schema.yaml`.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::Yaml`] if serialisation fails.
    pub fn to_yaml(&self) -> Result<String, SchemaError> {
        serde_yaml_ng::to_string(&self.to_raw()).map_err(|e| SchemaError::Yaml(e.to_string()))
    }

    /// A new schema with only a title property, for a fresh collection.
    #[must_use]
    pub fn new(id: CollectionId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            properties: vec![Property {
                key: "name".into(),
                name: "Name".into(),
                kind: PropertyType::Title,
            }],
            views: vec![ViewDef::default_table()],
        }
    }

    /// The property with this key.
    #[must_use]
    pub fn property(&self, key: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.key == key)
    }

    /// The title property.
    #[must_use]
    pub fn title_property(&self) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| matches!(p.kind, PropertyType::Title))
    }

    /// Finds a property by key, then exact name, then case-insensitive name.
    #[must_use]
    pub fn resolve(&self, name_or_key: &str) -> Option<&Property> {
        let wanted = name_or_key.trim();
        self.property(wanted)
            .or_else(|| self.properties.iter().find(|p| p.name == wanted))
            .or_else(|| {
                let lower = wanted.to_lowercase();
                self.properties
                    .iter()
                    .find(|p| p.name.to_lowercase() == lower)
            })
    }

    /// The view with this name (case-insensitive), or the first view.
    #[must_use]
    pub fn view(&self, name: Option<&str>) -> Option<&ViewDef> {
        name.map_or_else(
            || self.views.first(),
            |n| {
                let lower = n.trim().to_lowercase();
                self.views.iter().find(|v| v.name.to_lowercase() == lower)
            },
        )
    }

    pub(crate) fn from_raw(raw: SchemaRaw) -> Result<Self, SchemaError> {
        let id = raw.id.ok_or(SchemaError::MissingId)?;
        if raw.properties.len() > MAX_PROPERTIES {
            return Err(SchemaError::TooManyProperties(raw.properties.len()));
        }
        let mut properties = Vec::with_capacity(raw.properties.len());
        let mut keys = BTreeSet::new();
        let mut names = BTreeSet::new();
        for p in raw.properties {
            validate_key(&p.key)?;
            if !keys.insert(p.key.clone()) {
                return Err(SchemaError::DuplicateKey(p.key));
            }
            let name = p.name.trim().to_owned();
            if name.is_empty() {
                return Err(SchemaError::EmptyName(p.key));
            }
            if !names.insert(name.to_lowercase()) {
                return Err(SchemaError::DuplicateName(name));
            }
            let kind = property_type(&p)?;
            properties.push(Property {
                key: p.key,
                name,
                kind,
            });
        }
        let titles = properties
            .iter()
            .filter(|p| matches!(p.kind, PropertyType::Title))
            .count();
        if titles != 1 {
            return Err(SchemaError::TitleCount(titles));
        }
        let mut schema = Self {
            id,
            title: raw.title.unwrap_or_default(),
            properties,
            views: Vec::new(),
        };
        schema.compile_formulas()?;
        schema.check_local_refs()?;
        let mut views = Vec::with_capacity(raw.views.len());
        for v in raw.views {
            views.push(ViewDef::from_raw(&v, &schema)?);
        }
        if views.is_empty() {
            views.push(ViewDef::default_table());
        }
        schema.views = views;
        Ok(schema)
    }

    /// Compiles formulas against the property names, then rejects cycles
    /// between formulas.
    fn compile_formulas(&mut self) -> Result<(), SchemaError> {
        let compiled: Vec<Option<Formula>> = self
            .properties
            .iter()
            .map(|p| match &p.kind {
                PropertyType::Formula { formula, .. } => formula::compile(formula.source(), self)
                    .map(Some)
                    .map_err(|error| SchemaError::Formula {
                        key: p.key.clone(),
                        error,
                    }),
                _ => Ok(None),
            })
            .collect::<Result<_, _>>()?;
        for (p, f) in self.properties.iter_mut().zip(compiled) {
            if let (PropertyType::Formula { formula, .. }, Some(f)) = (&mut p.kind, f) {
                *formula = f;
            }
        }
        // Depth-first search for cycles among formula → formula references.
        let deps: BTreeMap<&str, Vec<&str>> = self
            .properties
            .iter()
            .filter_map(|p| match &p.kind {
                PropertyType::Formula { formula, .. } => Some((
                    p.key.as_str(),
                    formula.references().iter().map(String::as_str).collect(),
                )),
                _ => None,
            })
            .collect();
        let mut done = BTreeSet::new();
        for start in deps.keys() {
            let mut path = Vec::new();
            find_cycle(start, &deps, &mut path, &mut done)?;
        }
        Ok(())
    }

    fn check_local_refs(&self) -> Result<(), SchemaError> {
        for p in &self.properties {
            if let PropertyType::Rollup { via, .. } = &p.kind {
                match self.property(via) {
                    Some(Property {
                        kind: PropertyType::Relation { .. },
                        ..
                    }) => {}
                    _ => {
                        return Err(SchemaError::RollupVia {
                            key: p.key.clone(),
                            via: via.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn to_raw(&self) -> SchemaRaw {
        SchemaRaw {
            vellum: Some(1),
            id: Some(self.id),
            title: Some(self.title.clone()),
            properties: self.properties.iter().map(property_raw).collect(),
            views: self.views.iter().map(|v| v.to_raw(self)).collect(),
        }
    }
}

fn find_cycle<'a>(
    key: &'a str,
    deps: &BTreeMap<&'a str, Vec<&'a str>>,
    path: &mut Vec<&'a str>,
    done: &mut BTreeSet<&'a str>,
) -> Result<(), SchemaError> {
    if done.contains(key) {
        return Ok(());
    }
    if let Some(at) = path.iter().position(|k| *k == key) {
        let mut cycle: Vec<String> = path.iter().skip(at).map(|k| (*k).to_owned()).collect();
        cycle.push(key.to_owned());
        return Err(SchemaError::FormulaCycle(cycle));
    }
    path.push(key);
    if let Some(next) = deps.get(key) {
        for n in next {
            find_cycle(n, deps, path, done)?;
        }
    }
    let _ = path.pop();
    let _ = done.insert(key);
    Ok(())
}

fn validate_key(key: &str) -> Result<(), SchemaError> {
    let valid = !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(SchemaError::InvalidKey(key.to_owned()));
    }
    if RESERVED_KEYS.contains(&key) {
        return Err(SchemaError::ReservedKey(key.to_owned()));
    }
    Ok(())
}

fn property_type(p: &PropertyRaw) -> Result<PropertyType, SchemaError> {
    let missing = |field: &'static str| SchemaError::MissingField {
        key: p.key.clone(),
        field,
    };
    let format = p.format.unwrap_or_default();
    Ok(match p.kind.as_str() {
        "title" => PropertyType::Title,
        "text" => PropertyType::Text,
        "number" => PropertyType::Number { format },
        "select" | "multi" => {
            let mut seen = BTreeSet::new();
            let options: Vec<SelectOption> = p
                .options
                .iter()
                .map(OptionRaw::to_option)
                .filter(|o| seen.insert(o.name.clone()))
                .collect();
            if p.kind == "select" {
                PropertyType::Select { options }
            } else {
                PropertyType::Multi { options }
            }
        }
        "person" => PropertyType::Person,
        "date" => PropertyType::Date,
        "checkbox" => PropertyType::Checkbox,
        "url" => PropertyType::Url,
        "relation" => PropertyType::Relation {
            to: p.to.ok_or_else(|| missing("to"))?,
            back: p.back.clone(),
        },
        "rollup" => {
            let agg = p.agg.ok_or_else(|| missing("agg"))?;
            PropertyType::Rollup {
                via: p.via.clone().ok_or_else(|| missing("via"))?,
                prop: p.prop.clone().ok_or_else(|| missing("prop"))?,
                agg,
                format: p.format.unwrap_or(if agg == Aggregation::Pct {
                    NumberFormat::Percent
                } else {
                    NumberFormat::Plain
                }),
            }
        }
        "formula" => PropertyType::Formula {
            formula: Formula::uncompiled(p.f.clone().ok_or_else(|| missing("f"))?),
            format,
        },
        "created" => PropertyType::Created,
        other => {
            return Err(SchemaError::UnknownType {
                key: p.key.clone(),
                kind: other.to_owned(),
            });
        }
    })
}

fn property_raw(p: &Property) -> PropertyRaw {
    let mut raw = PropertyRaw {
        key: p.key.clone(),
        name: p.name.clone(),
        kind: p.kind.name().to_owned(),
        ..PropertyRaw::default()
    };
    let non_default = |f: NumberFormat| (f != NumberFormat::Plain).then_some(f);
    match &p.kind {
        PropertyType::Number { format } => raw.format = non_default(*format),
        PropertyType::Select { options } | PropertyType::Multi { options } => {
            raw.options = options.iter().map(OptionRaw::from_option).collect();
        }
        PropertyType::Relation { to, back } => {
            raw.to = Some(*to);
            raw.back.clone_from(back);
        }
        PropertyType::Rollup {
            via,
            prop,
            agg,
            format,
        } => {
            raw.via = Some(via.clone());
            raw.prop = Some(prop.clone());
            raw.agg = Some(*agg);
            let default = if *agg == Aggregation::Pct {
                NumberFormat::Percent
            } else {
                NumberFormat::Plain
            };
            raw.format = (*format != default).then_some(*format);
        }
        PropertyType::Formula { formula, format } => {
            raw.f = Some(formula.source().to_owned());
            raw.format = non_default(*format);
        }
        PropertyType::Title
        | PropertyType::Text
        | PropertyType::Person
        | PropertyType::Date
        | PropertyType::Checkbox
        | PropertyType::Url
        | PropertyType::Created => {}
    }
    raw
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SchemaRaw {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vellum: Option<u32>,
    #[serde(default)]
    id: Option<CollectionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default)]
    properties: Vec<PropertyRaw>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    views: Vec<ViewDefRaw>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertyRaw {
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    options: Vec<OptionRaw>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<CollectionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    back: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agg: Option<Aggregation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    f: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<NumberFormat>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OptionRaw {
    Name(String),
    Full(SelectOption),
}

impl OptionRaw {
    fn to_option(&self) -> SelectOption {
        match self {
            Self::Name(name) => SelectOption {
                name: name.clone(),
                color: None,
            },
            Self::Full(o) => o.clone(),
        }
    }

    fn from_option(o: &SelectOption) -> Self {
        if o.color.is_some() {
            Self::Full(o.clone())
        } else {
            Self::Name(o.name.clone())
        }
    }
}

/// The cheat sheet's requirements tracker, shared by unit tests.
#[cfg(test)]
pub(crate) const TEST_SCHEMA: &str = r#"
id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E
title: Requirements tracker
properties:
  - {key: t,  name: Requirement, type: title}
  - {key: pr, name: Priority, type: select, options: [P0, P1, {name: P2, color: gray}]}
  - {key: est, name: Estimate, type: number}
  - {key: prog, name: Progress, type: number, format: percent}
  - {key: f1, name: Remaining, type: formula, f: 'round(prop("Estimate") * (1 - Progress), 1)'}
views:
  - {name: All, type: table}
  - {name: By priority, type: board, group: pr}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = TEST_SCHEMA;

    #[test]
    fn parses_the_cheat_sheet_shape() {
        let s = Schema::from_yaml(SCHEMA).unwrap();
        assert_eq!(s.properties.len(), 5);
        assert_eq!(s.title_property().unwrap().key, "t");
        assert_eq!(s.resolve("priority").unwrap().key, "pr");
        assert_eq!(
            s.properties[1].kind.options()[2].color.as_deref(),
            Some("gray")
        );
        assert_eq!(s.views.len(), 2);
    }

    #[test]
    fn round_trips_through_yaml() {
        let s = Schema::from_yaml(SCHEMA).unwrap();
        let again = Schema::from_yaml(&s.to_yaml().unwrap()).unwrap();
        assert_eq!(s.to_yaml().unwrap(), again.to_yaml().unwrap());
    }

    #[test]
    fn rejects_bad_schemas() {
        let base = "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\nproperties:\n";
        let cases = [
            ("  - {key: a, name: A, type: text}\n", "title"),
            (
                "  - {key: t, name: T, type: title}\n  - {key: t, name: U, type: text}\n",
                "duplicate",
            ),
            ("  - {key: id, name: T, type: title}\n", "reserved"),
            ("  - {key: Bad, name: T, type: title}\n", "invalid"),
            (
                "  - {key: t, name: T, type: title}\n  - {key: x, name: X, type: wat}\n",
                "unknown",
            ),
            (
                "  - {key: t, name: T, type: title}\n  - {key: a, name: A, type: formula, f: 'B + 1'}\n  - {key: b, name: B, type: formula, f: 'A + 1'}\n",
                "circular",
            ),
            (
                "  - {key: t, name: T, type: title}\n  - {key: a, name: A, type: formula, f: 'Nope + 1'}\n",
                "unknown property",
            ),
            (
                "  - {key: t, name: T, type: title}\n  - {key: r, name: R, type: rollup, via: t, prop: x, agg: sum}\n",
                "relation",
            ),
        ];
        for (props, needle) in cases {
            let err = Schema::from_yaml(&format!("{base}{props}")).unwrap_err();
            assert!(
                err.to_string().contains(needle),
                "{err} should mention {needle}"
            );
        }
    }
}
