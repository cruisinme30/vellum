// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The safe formula language.
//!
//! | Syntax | Meaning |
//! |---|---|
//! | `prop("Due date")`, `Estimate` | a property by name (bare names must be one word) |
//! | `12`, `1.5`, `"text"`, `'text'`, `true` | literals |
//! | `+ - * / %` (also `× ÷ −`) | arithmetic; `+` joins text; `date ± n` moves days; `date - date` is days |
//! | `== != < > <= >=` (also `≠ ≤ ≥`) | comparisons |
//! | `and or not` | logic (short-circuit) |
//!
//! Functions: `if round len concat min max sum avg count now year days
//! format percent empty contains upper lower`.
//!
//! Formulas are computed on the server whenever a row is saved. They cannot
//! loop, call out, or read anything but the current row, so evaluation is
//! bounded and safe for untrusted input. Errors carry a [`Span`] so the UI
//! can show them inline in the cell and in the formula editor.

mod ast;
mod eval;
mod parse;

use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;

pub use ast::{BinOp, Expr, Func, Node, Span, UnOp};
pub use eval::{Env, MAX_STEPS, MAX_TEXT_BYTES};
pub(crate) use eval::{loose_eq, percent};
pub use parse::{MAX_DEPTH, MAX_SOURCE_CHARS, parse};

use crate::schema::Schema;
use crate::value::Value;

/// A formula error with the place it happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message} (at {}..{})", span.start, span.end)]
pub struct FormulaError {
    /// What went wrong, for people.
    pub message: String,
    /// Where in the formula source.
    pub span: Span,
}

impl FormulaError {
    pub(crate) fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

/// A formula compiled against a schema: property names resolved to keys.
#[derive(Debug, Clone)]
pub struct Formula {
    source: String,
    root: Option<Node>,
    refs: BTreeSet<String>,
}

impl Formula {
    pub(crate) const fn uncompiled(source: String) -> Self {
        Self {
            source,
            root: None,
            refs: BTreeSet::new(),
        }
    }

    /// The source as written in `schema.yaml`.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Keys of the properties the formula reads.
    #[must_use]
    pub const fn references(&self) -> &BTreeSet<String> {
        &self.refs
    }

    /// Evaluates the formula.
    ///
    /// # Errors
    ///
    /// Returns the first runtime error, such as a type mismatch or a
    /// division by zero, with its position in the source.
    pub fn eval(&self, env: &mut dyn Env) -> Result<Value, FormulaError> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| FormulaError::new("formula is not compiled", Span::default()))?;
        eval::Eval::new(env).eval(root)
    }
}

impl fmt::Display for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

/// Parses `source` and resolves property names against `schema`.
///
/// # Errors
///
/// Returns a syntax error, or an unknown-property error pointing at the
/// name.
pub fn compile(source: &str, schema: &Schema) -> Result<Formula, FormulaError> {
    let mut root = parse(source)?;
    let mut refs = BTreeSet::new();
    let mut error = None;
    root.walk_mut(&mut |node| {
        if error.is_some() {
            return;
        }
        if let Expr::Prop(name) = &mut node.expr {
            match schema.resolve(name) {
                Some(p) => {
                    let _ = refs.insert(p.key.clone());
                    name.clone_from(&p.key);
                }
                None => {
                    error = Some(FormulaError::new(
                        format!("unknown property {name:?}"),
                        node.span,
                    ));
                }
            }
        }
    });
    if let Some(e) = error {
        return Err(e);
    }
    Ok(Formula {
        source: source.to_owned(),
        root: Some(root),
        refs,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::value::DateValue;

    struct Row(BTreeMap<String, Value>);

    impl Env for Row {
        fn prop(&mut self, key: &str) -> Result<Value, String> {
            match key {
                "bad" => Err("Broken has an error".into()),
                _ => Ok(self.0.get(key).cloned().unwrap_or_default()),
            }
        }

        fn now(&self) -> DateValue {
            DateValue::parse("2026-10-29T08:00").unwrap()
        }
    }

    fn schema() -> Schema {
        Schema::from_yaml(
            r"
id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E
properties:
  - {key: t, name: Requirement, type: title}
  - {key: st, name: Status, type: select, options: [Todo, Done]}
  - {key: est, name: Estimate, type: number}
  - {key: prog, name: Progress, type: number}
  - {key: due, name: Target, type: date}
  - {key: team, name: Team, type: text}
  - {key: num, name: Number, type: number}
  - {key: pts, name: Points, type: multi}
  - {key: tags, name: Tags, type: multi}
  - {key: bad, name: Broken, type: number}
",
        )
        .unwrap()
    }

    fn run(src: &str) -> Result<Value, FormulaError> {
        let f = compile(src, &schema())?;
        let mut row = Row(BTreeMap::from([
            ("t".into(), Value::text("R1")),
            ("st".into(), Value::text("Todo")),
            ("est".into(), Value::Number(8.0)),
            ("prog".into(), Value::Number(0.3)),
            (
                "due".into(),
                Value::Date(DateValue::parse("2026-11-02").unwrap()),
            ),
            ("team".into(), Value::text("eng")),
            ("num".into(), Value::Number(142.0)),
            (
                "pts".into(),
                Value::List(vec![
                    Value::Number(3.0),
                    Value::Number(5.0),
                    Value::Number(8.0),
                ]),
            ),
            (
                "tags".into(),
                Value::List(vec![Value::text("ux"), Value::text("api")]),
            ),
        ]));
        f.eval(&mut row)
    }

    fn ok(src: &str) -> String {
        run(src).map_or_else(|e| format!("error: {e}"), |v| v.to_string())
    }

    #[test]
    fn cheat_sheet_examples() {
        assert_eq!(ok(r#"prop("Estimate") * (1 - Progress)"#), "5.6");
        assert_eq!(
            ok(
                r#"if(Status == "Done", "done", if(days(Target, now()) < 7, "due soon", "on track"))"#
            ),
            "due soon"
        );
        assert_eq!(
            ok(r#"round(sum(prop("Points")) / count(prop("Points")), 1)"#),
            "5.3"
        );
        assert_eq!(ok("percent(1 - 3.9 / 16)"), "76%");
        assert_eq!(ok(r#"concat(upper(Team), "-", format(Number))"#), "ENG-142");
    }

    #[test]
    fn every_function() {
        assert_eq!(ok("round(2.345, 2)"), "2.35");
        assert_eq!(ok("len(Requirement)"), "2");
        assert_eq!(ok("len(Tags)"), "2");
        assert_eq!(ok("min(3, Points, 1)"), "1");
        assert_eq!(ok("max(Points)"), "8");
        assert_eq!(ok("min(Target, now())"), "2026-10-29T08:00");
        assert_eq!(ok("avg(Points)"), "5.3333333333");
        assert!(ok("avg(Tags)").starts_with("error: avg needs numbers"));
        assert_eq!(ok("count(Tags, Requirement)"), "3");
        assert_eq!(ok("year(Target)"), "2026");
        assert_eq!(ok("days(Target, now())"), "4");
        assert_eq!(ok("Target - now()"), "4");
        assert_eq!(ok("Target + 30"), "2026-12-02");
        assert_eq!(ok("percent(0.756, 1)"), "75.6%");
        assert!(ok("empty(prop(\"Broken\") )").starts_with("error"));
        assert_eq!(ok("contains(Tags, \"ux\")"), "true");
        assert_eq!(ok("contains(Team, \"n\")"), "true");
        assert_eq!(ok("lower(\"ABC\")"), "abc");
        assert_eq!(ok("if(false, 1)"), "");
        assert_eq!(ok("not empty(Team) and Estimate >= 8 or false"), "true");
        assert_eq!(ok("\"n=\" + Estimate"), "n=8");
        assert_eq!(ok("7 % 4"), "3");
    }

    #[test]
    fn runtime_errors_are_positioned() {
        let e = run("1 + 2 / (Estimate - 8)").unwrap_err();
        assert_eq!(e.message, "division by zero");
        assert_eq!((e.span.start, e.span.end), (4, 22));
        let e = run("round(Team)").unwrap_err();
        assert!(e.message.contains("round needs a number, got text"));
        assert_eq!(e.span.start, 6);
        let e = run("Broken + 1").unwrap_err();
        assert_eq!(e.message, "Broken has an error");
        assert_eq!(e.span, Span::new(0, 6));
        let e = run("Team < 3").unwrap_err();
        assert!(e.message.contains("cannot compare text with number"));
    }

    #[test]
    fn short_circuit_skips_errors() {
        assert_eq!(ok("false and 1 / 0"), "false");
        assert_eq!(ok("if(true, 1, 1 / 0)"), "1");
    }

    #[test]
    fn unknown_property_is_a_compile_error() {
        let e = compile("Nope + 1", &schema()).unwrap_err();
        assert_eq!(e.span, Span::new(0, 4));
    }

    #[test]
    fn references_use_keys() {
        let f = compile(r#"Estimate + prop("Progress")"#, &schema()).unwrap();
        assert_eq!(
            f.references()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["est", "prog"]
        );
    }
}
