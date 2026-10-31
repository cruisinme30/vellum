// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Computed values: formulas, rollups and created times.
//!
//! A [`Computer`] evaluates lazily and memoises every cell, so a rollup
//! over a formula over another rollup is computed once per row however
//! many views ask for it. Cycles that cross collections (which schema
//! validation cannot see) become a "circular reference" cell error instead
//! of looping.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Instant;

use serde::Serialize;
use vellum_kernel::{CollectionId, DocId};

use crate::formula::{Env, FormulaError, Span, loose_eq};
use crate::schema::{Aggregation, PropertyType};
use crate::telemetry;
use crate::value::{DateValue, Value};
use crate::workspace::{RowRef, Workspace};

/// Why a computed cell has no value. Shown inline in the cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct CellError {
    /// What went wrong, for people.
    pub message: String,
    /// Where in the formula, when the error is in this cell's own formula.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl CellError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }
}

impl From<FormulaError> for CellError {
    fn from(e: FormulaError) -> Self {
        Self {
            message: e.message,
            span: Some(e.span),
        }
    }
}

/// A cell: a value or an inline error.
pub type Cell = Result<Value, CellError>;

/// Every property of one row, stored and computed, by key.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComputedRow {
    /// The row.
    pub id: DocId,
    /// Values by property key.
    #[serde(serialize_with = "serialize_cells")]
    #[schemars(with = "BTreeMap<String, CellValue>")]
    pub cells: BTreeMap<String, Cell>,
}

fn serialize_cells<S: serde::Serializer>(
    cells: &BTreeMap<String, Cell>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap as _;
    let mut map = serializer.serialize_map(Some(cells.len()))?;
    for (k, v) in cells {
        map.serialize_entry(k, &CellValue::from(v.clone()))?;
    }
    map.end()
}

/// The API shape of a [`Cell`]: `{"value": …}` or `{"error": …}`.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CellValue {
    /// A value.
    Value(Value),
    /// An inline error.
    Error(CellError),
}

impl From<Cell> for CellValue {
    fn from(c: Cell) -> Self {
        match c {
            Ok(v) => Self::Value(v),
            Err(e) => Self::Error(e),
        }
    }
}

type Key = (CollectionId, DocId, String);

/// Evaluates computed properties over a [`Workspace`].
pub struct Computer<'w> {
    ws: &'w Workspace,
    now: DateValue,
    memo: HashMap<Key, Cell>,
    active: HashSet<Key>,
}

impl<'w> Computer<'w> {
    /// A computer that evaluates `now()` as `now`.
    #[must_use]
    pub fn new(ws: &'w Workspace, now: DateValue) -> Self {
        Self {
            ws,
            now,
            memo: HashMap::new(),
            active: HashSet::new(),
        }
    }

    /// The workspace being computed.
    #[must_use]
    pub const fn workspace(&self) -> &'w Workspace {
        self.ws
    }

    /// The time `now()` returns.
    #[must_use]
    pub const fn now(&self) -> DateValue {
        self.now
    }

    /// One cell of one row.
    ///
    /// # Errors
    ///
    /// Returns the cell's inline error: a formula error, an error in a
    /// value it depends on, or a circular reference.
    pub fn value(
        &mut self,
        collection: CollectionId,
        row: DocId,
        key: &str,
    ) -> Cell {
        let k = (collection, row, key.to_owned());
        if let Some(c) = self.memo.get(&k) {
            return c.clone();
        }
        if !self.active.insert(k.clone()) {
            return Err(CellError::new("circular reference"));
        }
        let cell = self.compute(collection, row, key);
        let _ = self.active.remove(&k);
        if let Err(e) = &cell
            && e.span.is_some()
        {
            telemetry::formula_error();
        }
        let _ = self.memo.insert(k, cell.clone());
        cell
    }

    /// Every cell of one row.
    pub fn row(
        &mut self,
        collection: CollectionId,
        row: DocId,
    ) -> Option<ComputedRow> {
        let c = self.ws.collection(collection)?;
        let _ = c.row(row)?;
        let cells = c
            .schema()
            .properties
            .iter()
            .map(|p| (p.key.clone(), self.value(collection, row, &p.key)))
            .collect();
        Some(ComputedRow { id: row, cells })
    }

    /// Recomputes the given rows (from [`Workspace::affected`]), as the
    /// save path does.
    pub fn rows(
        &mut self,
        rows: &BTreeSet<RowRef>,
    ) -> Vec<(RowRef, ComputedRow)> {
        let span =
            tracing::info_span!("collections.compute", rows = rows.len());
        let _guard = span.enter();
        let started = Instant::now();
        let out: Vec<_> = rows
            .iter()
            .filter_map(|r| self.row(r.collection, r.row).map(|c| (*r, c)))
            .collect();
        telemetry::computed(out.len(), started);
        out
    }

    fn compute(
        &mut self,
        collection: CollectionId,
        row_id: DocId,
        key: &str,
    ) -> Cell {
        let ws = self.ws;
        let Some(c) = ws.collection(collection) else {
            return Err(CellError::new("collection not loaded"));
        };
        let Some(row) = c.row(row_id) else {
            return Err(CellError::new("row not found"));
        };
        let Some(p) = c.schema().property(key) else {
            return Err(CellError::new(format!("no property {key:?}")));
        };
        match &p.kind {
            PropertyType::Created => Ok(i64::try_from(row_id.timestamp_ms())
                .ok()
                .and_then(DateValue::from_unix_ms)
                .map_or(Value::Empty, Value::Date)),
            PropertyType::Formula { formula, .. } => {
                let mut env = RowEnv {
                    computer: self,
                    collection,
                    row: row_id,
                };
                formula.eval(&mut env).map_err(CellError::from)
            }
            PropertyType::Rollup { via, prop, agg, .. } => {
                let Some(PropertyType::Relation { to, .. }) =
                    c.schema().property(via).map(|v| &v.kind)
                else {
                    return Err(CellError::new(format!(
                        "{via:?} is not a relation"
                    )));
                };
                let to = *to;
                let Some(target) = ws.collection(to) else {
                    return Err(CellError::new(
                        "related collection not loaded",
                    ));
                };
                let prop_name = target
                    .schema()
                    .property(prop)
                    .map_or_else(|| prop.clone(), |p| p.name.clone());
                let mut values = Vec::new();
                for id in row.links(via) {
                    if target.row(id).is_none() {
                        continue;
                    }
                    match self.value(to, id, prop) {
                        Ok(v) => values.push(v),
                        Err(_) => {
                            return Err(CellError::new(format!(
                                "{prop_name} has an error on a related row"
                            )));
                        }
                    }
                }
                Ok(aggregate(*agg, &values))
            }
            // An unset checkbox is unchecked, never blank.
            PropertyType::Checkbox => Ok(Value::Bool(row.get(key).truthy())),
            _ => Ok(row.get(key).clone()),
        }
    }
}

struct RowEnv<'c, 'w> {
    computer: &'c mut Computer<'w>,
    collection: CollectionId,
    row: DocId,
}

impl Env for RowEnv<'_, '_> {
    fn prop(&mut self, key: &str) -> Result<Value, String> {
        self.computer
            .value(self.collection, self.row, key)
            .map_err(|e| {
                let name = self
                    .computer
                    .ws
                    .collection(self.collection)
                    .and_then(|c| c.schema().property(key))
                    .map_or_else(|| key.to_owned(), |p| p.name.clone());
                if e.message == "circular reference" {
                    e.message
                } else {
                    format!("{name} has an error")
                }
            })
    }

    fn now(&self) -> DateValue {
        self.computer.now
    }
}

#[allow(clippy::cast_precision_loss)]
const fn count(n: usize) -> Value {
    Value::Number(n as f64)
}

/// Combines related rows' values.
#[must_use]
pub fn aggregate(agg: Aggregation, values: &[Value]) -> Value {
    let leaves: Vec<&Value> = values.iter().flat_map(Value::leaves).collect();
    let numbers = || leaves.iter().filter_map(|v| v.as_number());
    match agg {
        Aggregation::Count => count(values.len()),
        Aggregation::Sum => Value::Number(numbers().sum()),
        Aggregation::Avg => {
            let n: Vec<f64> = numbers().collect();
            if n.is_empty() {
                Value::Empty
            } else {
                #[allow(clippy::cast_precision_loss)]
                let len = n.len() as f64;
                Value::Number(n.iter().sum::<f64>() / len)
            }
        }
        Aggregation::Min | Aggregation::Max => {
            let mut best: Option<&Value> = None;
            for v in leaves
                .iter()
                .filter(|v| matches!(v, Value::Number(_) | Value::Date(_)))
            {
                best = match best {
                    Some(b)
                        if std::mem::discriminant(b)
                            == std::mem::discriminant(*v)
                            && ((agg == Aggregation::Min)
                                == (v.sort_cmp(b).is_lt())) =>
                    {
                        Some(v)
                    }
                    Some(b) => Some(b),
                    None => Some(v),
                };
            }
            best.cloned().unwrap_or_default()
        }
        Aggregation::Checked => {
            count(values.iter().filter(|v| v.truthy()).count())
        }
        Aggregation::Pct => {
            if values.is_empty() {
                Value::Empty
            } else {
                #[allow(clippy::cast_precision_loss)]
                let pct = values.iter().filter(|v| v.truthy()).count() as f64
                    / values.len() as f64;
                Value::Number(pct)
            }
        }
        Aggregation::Unique => {
            let mut seen: Vec<&Value> = Vec::new();
            for v in &leaves {
                if !seen.iter().any(|s| loose_eq(s, v)) {
                    seen.push(v);
                }
            }
            count(seen.len())
        }
        Aggregation::Show => Value::List(leaves.into_iter().cloned().collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregations() {
        let vals = [
            Value::Number(3.0),
            Value::Number(5.0),
            Value::Empty,
            Value::Number(5.0),
        ];
        let run = |a| aggregate(a, &vals).to_string();
        assert_eq!(run(Aggregation::Count), "4");
        assert_eq!(run(Aggregation::Sum), "13");
        assert_eq!(run(Aggregation::Avg), "4.3333333333");
        assert_eq!(run(Aggregation::Min), "3");
        assert_eq!(run(Aggregation::Max), "5");
        assert_eq!(run(Aggregation::Unique), "2");
        assert_eq!(run(Aggregation::Show), "3, 5, 5");
        let checks = [
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Empty,
        ];
        assert_eq!(aggregate(Aggregation::Checked, &checks).to_string(), "2");
        assert_eq!(aggregate(Aggregation::Pct, &checks).to_string(), "0.5");
        assert_eq!(aggregate(Aggregation::Pct, &[]), Value::Empty);
        assert_eq!(aggregate(Aggregation::Avg, &[]), Value::Empty);
    }
}
