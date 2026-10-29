// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Calculator outputs, computed with the collections formula language so a
//! calculator shows the same numbers in the editor, on the page and in an
//! export.

use std::collections::HashMap;

use time::{Date, Month};
use vellum_collections::formula::{self, Env};
use vellum_collections::{DateValue, NumberFormat, Property, PropertyType, Schema, Value};
use vellum_format::blocks::{Calculator, CalculatorOutput};
use vellum_kernel::CollectionId;

/// An output's formula and unit. The format reads a trailing `(…)` as the
/// unit, which also matches a formula that ends in a function call such as
/// `round(x, 0)`; when the joined text is itself a formula, it is one.
fn formula_and_unit(output: &CalculatorOutput, schema: &Schema) -> (String, Option<String>) {
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

/// Computes each output from the stored input values, in order; later
/// outputs may use earlier ones. `as_of` (`YYYY-MM-DD`) answers `now()`.
/// Returns each output's unit with its value.
pub fn outputs(calc: &Calculator, as_of: &str) -> Vec<(Option<String>, Result<f64, String>)> {
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
            let result = formula::compile(&source, &schema)
                .and_then(|f| f.eval(&mut env))
                .map_err(|e| e.message)
                .and_then(|value| match value {
                    Value::Number(n) if n.is_finite() => Ok(n),
                    other => Err(format!("`{source}` is not a number ({other:?})")),
                });
            let stored = result.as_ref().map_or(Value::Empty, |n| Value::Number(*n));
            let _ = env.values.insert(output.key.clone(), stored);
            (unit, result)
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

/// Formats a number for display: integers without decimals, others with
/// up to two, thousands separated by commas.
#[must_use]
pub fn display_number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    let negative = rounded < 0.0;
    let text = format!("{:.2}", rounded.abs());
    let (int, frac) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let frac = frac.trim_end_matches('0');
    let sign = if negative { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

#[cfg(test)]
mod tests {
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
             revenue: Revenue = units * price ($)\nnet: Net = round(revenue * 0.8, 0)\n",
        );
        assert_eq!(
            outputs(&calc, "2026-10-29"),
            [(Some("$".into()), Ok(1250.0)), (None, Ok(1000.0))]
        );
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        let calc = calc("a: A = 1\nb: B = a / 0\nc: C = nope +\n");
        let out = outputs(&calc, "2026-10-29");
        assert!(out.iter().all(|(_, r)| r.is_err()), "{out:?}");
    }

    #[test]
    fn numbers_display_grouped_with_two_decimals_at_most() {
        assert_eq!(display_number(1_234_567.0), "1,234,567");
        assert_eq!(display_number(-0.125), "-0.13");
        assert_eq!(display_number(12.5), "12.5");
    }
}
