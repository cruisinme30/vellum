// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Collections inside pages: the `collection` fence and live chart sources.
//!
//! ````markdown
//! ```collection
//! id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E
//! view: By status
//! filter: Status != Done
//! ```
//!
//! ```chart {source=collection:01JBQ8Y4ZK6V3W2N5R7T9X1C3E x=Target y=Estimate}
//! ```
//! ````
//!
//! The renderer parses the fence and asks this module for the current
//! [`ViewResult`] or [`ChartData`]; exports freeze that state. Any error
//! comes back as an [`EmbedError`] whose message the renderer shows in a
//! readable fallback block, never as a failed page.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use vellum_kernel::CollectionId;

use crate::compute::Computer;
use crate::schema::{PropertyType, Schema};
use crate::value::Value;
use crate::view::{self, OneOrMany, ViewDef, ViewDefRaw, ViewKind, ViewResult};

/// Why an embed could not be shown.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EmbedError(pub String);

/// A parsed `collection` fence: which view to show, plus overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionEmbed {
    /// The collection.
    pub id: CollectionId,
    /// The saved view to start from (the first view when `None`).
    pub view: Option<String>,
    overrides: ViewDefRaw,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedRaw {
    id: String,
    #[serde(default)]
    view: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<ViewKind>,
    #[serde(default)]
    filter: OneOrMany,
    #[serde(default)]
    sort: OneOrMany,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    hide: OneOrMany,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

fn parse_collection_id(text: &str) -> Result<CollectionId, EmbedError> {
    let text = text.trim();
    let text = text.strip_prefix("vellum://collection/").unwrap_or(text);
    text.parse()
        .map_err(|e| EmbedError(format!("collection id: {e}")))
}

impl CollectionEmbed {
    /// Parses the fence body (YAML).
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedError`] for invalid YAML, unknown keys or a bad id.
    pub fn parse(body: &str) -> Result<Self, EmbedError> {
        let raw: EmbedRaw = crate::yaml::from_str(body)
            .map_err(|e| EmbedError(format!("collection block: {e}")))?;
        Ok(Self {
            id: parse_collection_id(&raw.id)?,
            view: raw.view,
            overrides: ViewDefRaw {
                name: None,
                kind: raw.kind,
                filter: raw.filter,
                sort: raw.sort,
                group: raw.group,
                hide: raw.hide,
                date: raw.date,
                end: raw.end,
                limit: raw.limit,
            },
        })
    }

    /// The effective view: the saved view with the fence's overrides.
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedError`] when the view or an override is unknown.
    pub fn view_def(&self, schema: &Schema) -> Result<ViewDef, EmbedError> {
        let mut def = schema.view(self.view.as_deref()).cloned().ok_or_else(|| {
            EmbedError(format!(
                "{} has no view named {:?}",
                schema.title,
                self.view.as_deref().unwrap_or_default()
            ))
        })?;
        def.overlay(&self.overrides, schema)
            .map_err(|e| EmbedError(e.to_string()))?;
        Ok(def)
    }

    /// Runs the embedded view.
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedError`] when the collection is not available or
    /// the view is invalid.
    pub fn resolve(&self, computer: &mut Computer<'_>) -> Result<ViewResult, EmbedError> {
        let c = computer
            .workspace()
            .collection(self.id)
            .ok_or_else(|| EmbedError("this collection is not available".into()))?;
        let def = self.view_def(c.schema())?;
        view::query(computer, self.id, &def).map_err(|e| EmbedError(e.to_string()))
    }
}

/// How a chart combines the `y` values that share an `x`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChartAgg {
    /// Add them (the default when `y` is given).
    #[default]
    Sum,
    /// Count rows (the default without `y`).
    Count,
    /// Mean.
    Avg,
    /// Smallest.
    Min,
    /// Largest.
    Max,
}

/// A chart's live data source: `source=collection:<id> x=… y=…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartSource {
    /// The collection.
    pub id: CollectionId,
    /// The view whose rows feed the chart (the first view when `None`).
    pub view: Option<String>,
    /// The category property.
    pub x: String,
    /// The value property; rows are counted when `None`.
    pub y: Option<String>,
    /// An optional property splitting rows into series.
    pub series: Option<String>,
    /// How values that share a category combine.
    pub agg: ChartAgg,
}

/// One chart series.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Series {
    /// The legend label.
    pub name: String,
    /// One value per category.
    pub values: Vec<f64>,
}

/// Data for a chart, ready for the renderer.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChartData {
    /// The x-axis label.
    pub x_label: String,
    /// The y-axis label.
    pub y_label: String,
    /// The categories, in axis order.
    pub categories: Vec<String>,
    /// The series.
    pub series: Vec<Series>,
}

impl ChartSource {
    /// Reads the source from a chart fence's attributes. Returns `Ok(None)`
    /// when `source` is not a collection.
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedError`] for a bad id, missing `x` or unknown `agg`.
    pub fn from_attrs(attrs: &BTreeMap<String, String>) -> Result<Option<Self>, EmbedError> {
        let Some(source) = attrs.get("source") else {
            return Ok(None);
        };
        let Some(id) = source.strip_prefix("collection:") else {
            return Ok(None);
        };
        let x = attrs
            .get("x")
            .cloned()
            .ok_or_else(|| EmbedError("a collection chart needs x=<property>".into()))?;
        let y = attrs.get("y").cloned();
        let agg = match attrs.get("agg").map(|a| a.to_ascii_lowercase()) {
            None if y.is_none() => ChartAgg::Count,
            None => ChartAgg::Sum,
            Some(a) => match a.as_str() {
                "sum" => ChartAgg::Sum,
                "count" => ChartAgg::Count,
                "avg" => ChartAgg::Avg,
                "min" => ChartAgg::Min,
                "max" => ChartAgg::Max,
                other => return Err(EmbedError(format!("unknown agg {other:?}"))),
            },
        };
        Ok(Some(Self {
            id: parse_collection_id(id)?,
            view: attrs.get("view").cloned(),
            x,
            y,
            series: attrs.get("series").cloned(),
            agg,
        }))
    }

    /// Computes the chart data from the view's current rows.
    ///
    /// # Errors
    ///
    /// Returns an [`EmbedError`] when the collection, view or a property is
    /// unknown.
    pub fn resolve(&self, computer: &mut Computer<'_>) -> Result<ChartData, EmbedError> {
        let ws = computer.workspace();
        let schema = ws
            .collection(self.id)
            .ok_or_else(|| EmbedError("this collection is not available".into()))?
            .schema();
        let prop = |name: &str| {
            schema
                .resolve(name)
                .ok_or_else(|| EmbedError(format!("{} has no property {name:?}", schema.title)))
        };
        let x = prop(&self.x)?;
        let y = self.y.as_deref().map(prop).transpose()?;
        let series_prop = self.series.as_deref().map(prop).transpose()?;
        let def = schema.view(self.view.as_deref()).cloned().ok_or_else(|| {
            EmbedError(format!(
                "{} has no view named {:?}",
                schema.title, self.view
            ))
        })?;
        let result = view::run(computer, self.id, &def, usize::MAX)
            .map_err(|e| EmbedError(e.to_string()))?;

        // (series, category) -> values
        let mut cells: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
        let mut categories: Vec<(Value, String)> = Vec::new();
        let mut series_names: Vec<String> = Vec::new();
        let mut seen = BTreeSet::new();
        for row in result.groups.iter().flat_map(|g| &g.rows) {
            if !seen.insert(row.id) {
                continue;
            }
            let xv = computer.value(self.id, row.id, &x.key).unwrap_or_default();
            let xs = labelled(computer, x, &xv);
            let n = match (y, self.agg) {
                (_, ChartAgg::Count) | (None, _) => Some(1.0),
                (Some(y), _) => computer
                    .value(self.id, row.id, &y.key)
                    .ok()
                    .and_then(|v| v.as_number()),
            };
            let Some(n) = n else { continue };
            let names: Vec<String> = series_prop.map_or_else(
                || vec![y.map_or_else(|| "Count".to_owned(), |y| y.name.clone())],
                |s| {
                    let sv = computer.value(self.id, row.id, &s.key).unwrap_or_default();
                    labelled(computer, s, &sv)
                        .into_iter()
                        .map(|(_, l)| l)
                        .collect()
                },
            );
            for (v, label) in &xs {
                if !categories.iter().any(|(_, l)| l == label) {
                    categories.push((v.clone(), label.clone()));
                }
                for s in &names {
                    if !series_names.contains(s) {
                        series_names.push(s.clone());
                    }
                    cells.entry((s.clone(), label.clone())).or_default().push(n);
                }
            }
        }
        categories.sort_by(|(a, _), (b, _)| view::compare_for_sort(Some(x), a, b));
        let categories: Vec<String> = categories.into_iter().map(|(_, l)| l).collect();
        let series = series_names
            .into_iter()
            .map(|name| Series {
                values: categories
                    .iter()
                    .map(|cat| combine(self.agg, cells.get(&(name.clone(), cat.clone()))))
                    .collect(),
                name,
            })
            .collect();
        Ok(ChartData {
            x_label: x.name.clone(),
            y_label: y.map_or_else(|| "Count".to_owned(), |y| y.name.clone()),
            categories,
            series,
        })
    }
}

/// Category values with their labels; an empty value is its own category.
fn labelled(
    computer: &Computer<'_>,
    p: &crate::schema::Property,
    v: &Value,
) -> Vec<(Value, String)> {
    if v.is_empty() {
        return vec![(Value::Empty, format!("No {}", p.name))];
    }
    let texts = view::item_texts(computer, p, v);
    let values: Vec<Value> = match &p.kind {
        PropertyType::Relation { .. } => v.items().to_vec(),
        _ => v.leaves().into_iter().cloned().collect(),
    };
    values.into_iter().zip(texts).collect()
}

fn combine(agg: ChartAgg, values: Option<&Vec<f64>>) -> f64 {
    let Some(values) = values.filter(|v| !v.is_empty()) else {
        return 0.0;
    };
    match agg {
        ChartAgg::Sum | ChartAgg::Count => values.iter().sum(),
        ChartAgg::Avg => {
            #[allow(clippy::cast_precision_loss)]
            let n = values.len() as f64;
            values.iter().sum::<f64>() / n
        }
        ChartAgg::Min => values.iter().copied().fold(f64::INFINITY, f64::min),
        ChartAgg::Max => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_fence() {
        let e = CollectionEmbed::parse(
            "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\nview: By status\nfilter: Status != Done\n",
        )
        .unwrap();
        assert_eq!(e.view.as_deref(), Some("By status"));
        assert!(CollectionEmbed::parse("id: nope").is_err());
        assert!(CollectionEmbed::parse("id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\nbogus: 1").is_err());
    }

    #[test]
    fn parses_chart_attrs() {
        let attrs: BTreeMap<String, String> = [
            ("source", "collection:01JBQ8Y4ZK6V3W2N5R7T9X1C3E"),
            ("x", "Target"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let s = ChartSource::from_attrs(&attrs).unwrap().unwrap();
        assert_eq!(s.agg, ChartAgg::Count);
        let none: BTreeMap<String, String> = BTreeMap::new();
        assert!(ChartSource::from_attrs(&none).unwrap().is_none());
    }
}
