// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The collections engine behind export and import.
//!
//! Export and collections are both services, so export cannot call
//! collections directly. It asks the [`Collections`] port instead, and
//! [`CollectionsEngine`] answers it here, where both are in reach:
//! calculators are computed with the collections formula language, and
//! imported databases are written as collection folders.

use std::collections::HashMap;

use time::{Date, Month};
use vellum_collections::csv::import_new;
use vellum_collections::formula::{self, Env};
use vellum_collections::{
    Collection, DateValue, Edit, NumberFormat, Property, PropertyType, Schema,
    Value, Workspace,
};
use vellum_export::ports::{
    CalcValue, CollectionFiles, Collections, NewCollection,
};
use vellum_format::blocks::{Calculator, CalculatorOutput};
use vellum_kernel::{CollectionId, DocId};

/// Answers export's [`Collections`] port with the collections engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct CollectionsEngine;

impl Collections for CollectionsEngine {
    fn calculate(&self, calc: &Calculator, as_of: &str) -> Vec<CalcValue> {
        outputs(calc, as_of)
    }

    fn create(&self, table: &NewCollection) -> Result<CollectionFiles, String> {
        create(table)
    }
}

/// An output's formula and unit. The format reads a trailing `(…)` as the
/// unit, which also matches a formula that ends in a function call such as
/// `round(x, 0)`; when the joined text is itself a formula, it is one.
fn formula_and_unit(
    output: &CalculatorOutput,
    schema: &Schema,
) -> (String, Option<String>) {
    if let Some(unit) = &output.unit {
        let joined = format!("{}({unit})", output.formula);
        if output.formula.ends_with(|c: char| c.is_alphanumeric())
            && formula::compile(&joined, schema).is_ok()
        {
            return (joined, None);
        }
    }
    (output.formula.clone(), output.unit.clone())
}

fn outputs(calc: &Calculator, as_of: &str) -> Vec<CalcValue> {
    let mut schema = Schema::new(CollectionId::from_u128(0), "calculator");
    for key in calc
        .inputs
        .iter()
        .map(|i| &i.key)
        .chain(calc.outputs.iter().map(|o| &o.key))
    {
        schema.properties.push(Property {
            key: key.clone(),
            name: key.clone(),
            kind: PropertyType::Number {
                format: NumberFormat::Plain,
            },
        });
    }
    let mut env = Values {
        values: calc
            .inputs
            .iter()
            .map(|i| (i.key.clone(), Value::Number(i.value)))
            .collect(),
        now: DateValue::day(parse_day(as_of).unwrap_or(Date::MIN)),
    };
    calc.outputs
        .iter()
        .map(|output| {
            let (source, unit) = formula_and_unit(output, &schema);
            let value = formula::compile(&source, &schema)
                .and_then(|f| f.eval(&mut env))
                .map_err(|e| e.message)
                .and_then(|value| match value {
                    Value::Number(n) if n.is_finite() => Ok(n),
                    other => {
                        Err(format!("`{source}` is not a number ({other:?})"))
                    }
                });
            let stored =
                value.as_ref().map_or(Value::Empty, |n| Value::Number(*n));
            let _ = env.values.insert(output.key.clone(), stored);
            CalcValue { unit, value }
        })
        .collect()
}

struct Values {
    values: HashMap<String, Value>,
    now: DateValue,
}

impl Env for Values {
    fn prop(&mut self, key: &str) -> Result<Value, String> {
        Ok(self.values.get(key).cloned().unwrap_or(Value::Empty))
    }

    fn now(&self) -> DateValue {
        self.now
    }
}

fn parse_day(text: &str) -> Option<Date> {
    let mut parts = text.splitn(3, '-');
    let year = parts.next()?.parse().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.get(..2)?.parse().ok()?;
    Date::from_calendar_date(year, month, day).ok()
}

/// Builds a collection from a table and its row pages. Rows are matched to
/// pages by title, ignoring case.
fn create(table: &NewCollection) -> Result<CollectionFiles, String> {
    let id = CollectionId::generate();
    let read =
        import_new(&table.csv, id, &table.title).map_err(|e| e.to_string())?;
    let mut problems: Vec<String> = read
        .diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.location, d.message))
        .collect();
    let title_key = read
        .schema
        .properties
        .iter()
        .find(|p| matches!(p.kind, PropertyType::Title))
        .map(|p| p.key.clone())
        .unwrap_or_default();
    let mut bodies: HashMap<String, (&str, &str)> = table
        .rows
        .iter()
        .map(|row| {
            (
                row.title.to_lowercase(),
                (row.id.as_str(), row.body.as_str()),
            )
        })
        .collect();
    let mut workspace = Workspace::new();
    workspace
        .insert(Collection::new(read.schema.clone()))
        .map_err(|e| e.to_string())?;
    for values in &read.rows {
        let title = values
            .get(&title_key)
            .map(|v| format!("{v}").to_lowercase())
            .unwrap_or_default();
        let (doc, body) = bodies
            .remove(&title)
            .map_or((None, String::new()), |(doc, body)| {
                (doc.parse::<DocId>().ok(), body.to_owned())
            });
        if let Err(error) = workspace.apply(Edit::CreateRow {
            collection: id,
            id: doc,
            values: values.clone(),
            body,
        }) {
            problems.push(format!("row `{title}` skipped: {error}"));
        }
    }
    for (title, _) in bodies {
        problems.push(format!("row page `{title}` has no row in the table"));
    }
    let files = workspace
        .collection(id)
        .ok_or("the collection was not created")?
        .files()
        .map_err(|e| e.to_string())?;
    Ok(CollectionFiles { files, problems })
}

#[cfg(test)]
mod tests {
    use vellum_export::ports::RowPage;
    use vellum_format::{Attributes, Fence};

    use super::*;

    fn calc(body: &str) -> Calculator {
        Calculator::parse(&Fence {
            id: None,
            lang: "calculator".into(),
            args: String::new(),
            attrs: Attributes::default(),
            literal: body.into(),
        })
        .data
    }

    #[test]
    fn outputs_use_inputs_and_earlier_outputs() {
        let calc = calc(
            "units: Units = 100 [0..1000 step 10]\nprice: Price = 12.5\n\
             revenue: Revenue = units * price ($)\n\
             net: Net = round(revenue * 0.8, 0)\n",
        );
        assert_eq!(
            CollectionsEngine.calculate(&calc, "2026-10-29"),
            [
                CalcValue {
                    unit: Some("$".into()),
                    value: Ok(1250.0)
                },
                CalcValue {
                    unit: None,
                    value: Ok(1000.0)
                }
            ]
        );
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        let calc = calc("a: A = 1\nb: B = a / 0\nc: C = nope +\n");
        let out = CollectionsEngine.calculate(&calc, "2026-10-29");
        assert!(out.iter().all(|v| v.value.is_err()), "{out:?}");
    }

    #[test]
    fn tables_become_collections_with_row_bodies() {
        let id = DocId::generate().to_string();
        let built = CollectionsEngine
            .create(&NewCollection {
                title: "Tasks".into(),
                csv: "Name,Status\nShip it,Done\nPlan,Todo\n".into(),
                rows: vec![RowPage {
                    title: "Ship it".into(),
                    id: id.clone(),
                    body: "The release notes are drafted.\n".into(),
                }],
            })
            .unwrap();
        let paths: Vec<&str> =
            built.files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"schema.yaml"), "{paths:?}");
        assert_eq!(paths.iter().filter(|p| p.starts_with("rows/")).count(), 2);
        let shipped = built
            .files
            .iter()
            .find(|(_, text)| text.contains("release notes"))
            .unwrap();
        assert!(shipped.1.contains(&id), "{}", shipped.1);
        assert!(built.problems.is_empty(), "{:?}", built.problems);
    }
}
