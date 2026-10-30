// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The evaluator.
//!
//! Evaluation is total: every input either yields a [`Value`] or a
//! [`FormulaError`] pointing at the sub-expression that failed. There are
//! no loops or user-defined functions, and a step budget bounds the work,
//! so a formula always finishes quickly.

use std::cmp::Ordering;

use super::FormulaError;
use super::ast::{BinOp, Expr, Func, Node, Span, UnOp};
use crate::value::{DateValue, Value, format_number};

/// Most nodes one evaluation may visit.
pub const MAX_STEPS: usize = 100_000;

/// Longest text a formula may produce, in bytes.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// What a formula can see while it runs.
pub trait Env {
    /// The value of the property with this key on the current row.
    ///
    /// # Errors
    ///
    /// Returns an error when the property's own value is an error.
    fn prop(&mut self, key: &str) -> Result<Value, String>;

    /// The current time, for `now()`.
    fn now(&self) -> DateValue;
}

pub(super) struct Eval<'e> {
    env: &'e mut dyn Env,
    steps: usize,
}

type R = Result<Value, FormulaError>;

impl<'e> Eval<'e> {
    pub(super) fn new(env: &'e mut dyn Env) -> Self {
        Self { env, steps: 0 }
    }

    pub(super) fn eval(&mut self, node: &Node) -> R {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(FormulaError::new(
                "formula does too much work",
                node.span,
            ));
        }
        let v = match &node.expr {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(t) => Value::Text(t.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Prop(key) => self
                .env
                .prop(key)
                .map_err(|m| FormulaError::new(m, node.span))?,
            Expr::Unary(UnOp::Neg, e) => {
                let v = self.eval(e)?;
                match v {
                    Value::Number(n) => Value::Number(-n),
                    Value::Empty => Value::Empty,
                    other => {
                        return Err(type_error(
                            "-", "a number", &other, e.span,
                        ));
                    }
                }
            }
            Expr::Unary(UnOp::Not, e) => Value::Bool(!self.eval(e)?.truthy()),
            Expr::Binary(BinOp::And, l, r) => {
                Value::Bool(self.eval(l)?.truthy() && self.eval(r)?.truthy())
            }
            Expr::Binary(BinOp::Or, l, r) => {
                Value::Bool(self.eval(l)?.truthy() || self.eval(r)?.truthy())
            }
            Expr::Binary(op, l, r) => {
                let a = self.eval(l)?;
                let b = self.eval(r)?;
                binary(*op, &a, &b, node.span)?
            }
            Expr::Call(func, args) => self.call(*func, args, node.span)?,
        };
        check(v, node.span)
    }

    fn call(&mut self, func: Func, args: &[Node], span: Span) -> R {
        // `if` is lazy; every other function evaluates all its arguments.
        if func == Func::If {
            let cond = self.eval(arg(args, 0, span)?)?;
            return match (cond.truthy(), args.get(2)) {
                (true, _) => self.eval(arg(args, 1, span)?),
                (false, Some(e)) => self.eval(e),
                (false, None) => Ok(Value::Empty),
            };
        }
        let vals: Vec<Value> = args
            .iter()
            .map(|a| self.eval(a))
            .collect::<Result<_, _>>()?;
        let first = vals.first().cloned().unwrap_or_default();
        let arg_span = |i: usize| args.get(i).map_or(span, |a| a.span);
        Ok(match func {
            Func::If => Value::Empty,
            Func::Round => {
                let digits = digits_arg(vals.get(1), arg_span(1))?;
                match first {
                    Value::Empty => Value::Empty,
                    Value::Number(n) => {
                        let scale = 10f64.powi(digits);
                        Value::Number((n * scale).round() / scale)
                    }
                    other => {
                        return Err(type_error(
                            "round",
                            "a number",
                            &other,
                            arg_span(0),
                        ));
                    }
                }
            }
            Func::Len => match first {
                Value::Text(t) => count(t.chars().count()),
                Value::List(items) => {
                    count(items.iter().filter(|v| !v.is_empty()).count())
                }
                Value::Empty => count(0),
                other => count(other.to_string().chars().count()),
            },
            Func::Concat => {
                let mut out = String::new();
                for v in &vals {
                    out.push_str(&v.to_string());
                    if out.len() > MAX_TEXT_BYTES {
                        return Err(too_long(span));
                    }
                }
                Value::Text(out)
            }
            Func::Min | Func::Max => extreme(func, &vals, span)?,
            Func::Sum => {
                Value::Number(numbers(&vals, args, "sum")?.iter().sum())
            }
            Func::Avg => {
                let ns = numbers(&vals, args, "avg")?;
                if ns.is_empty() {
                    Value::Empty
                } else {
                    Value::Number(ns.iter().sum::<f64>() / len_f64(ns.len()))
                }
            }
            Func::Count => count(vals.iter().map(|v| v.leaves().len()).sum()),
            Func::Now => Value::Date(self.env.now()),
            Func::Year => match first {
                Value::Date(d) => Value::Number(f64::from(d.date().year())),
                Value::Empty => Value::Empty,
                other => {
                    return Err(type_error(
                        "year",
                        "a date",
                        &other,
                        arg_span(0),
                    ));
                }
            },
            Func::Days => match (&first, vals.get(1)) {
                (Value::Date(a), Some(Value::Date(b))) => {
                    days_value(a.days_since(*b))
                }
                (Value::Empty, _) | (_, Some(Value::Empty) | None) => {
                    Value::Empty
                }
                (Value::Date(_), Some(other)) => {
                    return Err(type_error(
                        "days",
                        "two dates",
                        other,
                        arg_span(1),
                    ));
                }
                (other, _) => {
                    return Err(type_error(
                        "days",
                        "two dates",
                        other,
                        arg_span(0),
                    ));
                }
            },
            Func::Format => Value::Text(first.to_string()),
            Func::Percent => {
                let digits = digits_arg(vals.get(1), arg_span(1))?;
                match first {
                    Value::Empty => Value::Empty,
                    Value::Number(n) => Value::Text(percent(n, digits)),
                    other => {
                        return Err(type_error(
                            "percent",
                            "a number",
                            &other,
                            arg_span(0),
                        ));
                    }
                }
            }
            Func::Empty => Value::Bool(first.is_empty()),
            Func::Contains => {
                let needle = vals.get(1).cloned().unwrap_or_default();
                Value::Bool(match &first {
                    Value::List(items) => {
                        items.iter().any(|i| loose_eq(i, &needle))
                    }
                    Value::Empty => false,
                    hay => hay.to_string().contains(&needle.to_string()),
                })
            }
            Func::Upper => text_map(first, str::to_uppercase),
            Func::Lower => text_map(first, str::to_lowercase),
        })
    }
}

fn arg(args: &[Node], i: usize, span: Span) -> Result<&Node, FormulaError> {
    args.get(i)
        .ok_or_else(|| FormulaError::new("missing argument", span))
}

fn check(v: Value, span: Span) -> R {
    match &v {
        Value::Number(n) if !n.is_finite() => {
            Err(FormulaError::new("number is out of range", span))
        }
        Value::Text(t) if t.len() > MAX_TEXT_BYTES => Err(too_long(span)),
        _ => Ok(v),
    }
}

fn too_long(span: Span) -> FormulaError {
    FormulaError::new("text result is longer than 64 KiB", span)
}

fn type_error(what: &str, want: &str, got: &Value, span: Span) -> FormulaError {
    FormulaError::new(
        format!("{what} needs {want}, got {}", got.type_name()),
        span,
    )
}

#[allow(clippy::cast_precision_loss)]
const fn len_f64(n: usize) -> f64 {
    n as f64
}

const fn count(n: usize) -> Value {
    Value::Number(len_f64(n))
}

#[allow(clippy::cast_precision_loss)]
const fn days_value(d: i64) -> Value {
    Value::Number(d as f64)
}

fn digits_arg(v: Option<&Value>, span: Span) -> Result<i32, FormulaError> {
    match v {
        None | Some(Value::Empty) => Ok(0),
        Some(Value::Number(n))
            if n.fract() == 0.0 && (0.0..=10.0).contains(n) =>
        {
            // Checked to be a whole number in 0..=10 above.
            #[allow(clippy::cast_possible_truncation)]
            let d = *n as i32;
            Ok(d)
        }
        Some(_) => Err(FormulaError::new(
            "digits must be a whole number from 0 to 10",
            span,
        )),
    }
}

/// `0.756` → `"76%"`, or `"75.6%"` with one digit.
pub fn percent(n: f64, digits: i32) -> String {
    let scale = 10f64.powi(digits);
    format!("{}%", format_number((n * 100.0 * scale).round() / scale))
}

fn text_map(v: Value, f: impl Fn(&str) -> String) -> Value {
    match v {
        Value::Empty => Value::Empty,
        Value::Text(t) => Value::Text(f(&t)),
        other => Value::Text(f(&other.to_string())),
    }
}

fn numbers(
    vals: &[Value],
    args: &[Node],
    what: &str,
) -> Result<Vec<f64>, FormulaError> {
    let mut out = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        for leaf in v.leaves() {
            match leaf {
                Value::Number(n) => out.push(*n),
                other => {
                    let span = args.get(i).map(|a| a.span).unwrap_or_default();
                    return Err(type_error(what, "numbers", other, span));
                }
            }
        }
    }
    Ok(out)
}

fn extreme(func: Func, vals: &[Value], span: Span) -> R {
    let mut best: Option<&Value> = None;
    for leaf in vals.iter().flat_map(Value::leaves) {
        if !matches!(leaf, Value::Number(_) | Value::Date(_)) {
            return Err(type_error(
                func.name(),
                "numbers or dates",
                leaf,
                span,
            ));
        }
        best = match best {
            None => Some(leaf),
            Some(b) => {
                let ord = ordered(b, leaf, span)?;
                let take = if func == Func::Min {
                    ord == Ordering::Greater
                } else {
                    ord == Ordering::Less
                };
                Some(if take { leaf } else { b })
            }
        };
    }
    Ok(best.cloned().unwrap_or_default())
}

/// Equality used by `==`, `!=` and `contains`: empty equals blank text,
/// numbers compare numerically, everything else structurally.
pub fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (x, y) if x.is_empty() && y.is_empty() => true,
        (Value::Number(x), Value::Number(y)) => {
            (x - y).abs() < f64::EPSILON * x.abs().max(1.0)
        }
        _ => a == b,
    }
}

fn ordered(a: &Value, b: &Value, span: Span) -> Result<Ordering, FormulaError> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Ok(x.total_cmp(y)),
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Text(x), Value::Text(y)) => Ok(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ => Err(FormulaError::new(
            format!("cannot compare {} with {}", a.type_name(), b.type_name()),
            span,
        )),
    }
}

const fn num_or_zero(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Empty => Some(0.0),
        _ => None,
    }
}

fn binary(op: BinOp, a: &Value, b: &Value, span: Span) -> R {
    let arith_err = |a: &Value, b: &Value| {
        FormulaError::new(
            format!(
                "cannot apply {} to {} and {}",
                match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    _ => "%",
                },
                a.type_name(),
                b.type_name()
            ),
            span,
        )
    };
    match op {
        BinOp::Eq => Ok(Value::Bool(loose_eq(a, b))),
        BinOp::Ne => Ok(Value::Bool(!loose_eq(a, b))),
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            if a.is_empty() || b.is_empty() {
                return Ok(Value::Bool(false));
            }
            let ord = ordered(a, b, span)?;
            Ok(Value::Bool(match op {
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Gt => ord == Ordering::Greater,
                BinOp::Le => ord != Ordering::Greater,
                _ => ord != Ordering::Less,
            }))
        }
        BinOp::Add => match (a, b) {
            (Value::Text(_), _) | (_, Value::Text(_)) => {
                Ok(Value::Text(format!("{a}{b}")))
            }
            (Value::Date(d), x) | (x, Value::Date(d)) => {
                date_shift(*d, x, 1, span)
            }
            _ => match (num_or_zero(a), num_or_zero(b)) {
                (Some(x), Some(y)) => Ok(Value::Number(x + y)),
                _ => Err(arith_err(a, b)),
            },
        },
        BinOp::Sub => match (a, b) {
            (Value::Date(x), Value::Date(y)) => {
                Ok(days_value(x.days_since(*y)))
            }
            (Value::Date(d), x) => date_shift(*d, x, -1, span),
            _ => match (num_or_zero(a), num_or_zero(b)) {
                (Some(x), Some(y)) => Ok(Value::Number(x - y)),
                _ => Err(arith_err(a, b)),
            },
        },
        BinOp::Mul | BinOp::Div | BinOp::Rem => {
            let (Some(x), Some(y)) = (num_or_zero(a), num_or_zero(b)) else {
                return Err(arith_err(a, b));
            };
            if op != BinOp::Mul && y == 0.0 {
                return Err(FormulaError::new("division by zero", span));
            }
            Ok(Value::Number(match op {
                BinOp::Mul => x * y,
                BinOp::Div => x / y,
                _ => x % y,
            }))
        }
        BinOp::And | BinOp::Or => Ok(Value::Bool(match op {
            BinOp::And => a.truthy() && b.truthy(),
            _ => a.truthy() || b.truthy(),
        })),
    }
}

fn date_shift(d: DateValue, by: &Value, sign: i64, span: Span) -> R {
    match by {
        Value::Empty => Ok(Value::Date(d)),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e7 => {
            // Bounded above, so the cast is exact.
            #[allow(clippy::cast_possible_truncation)]
            let days = *n as i64;
            d.add_days(sign * days)
                .map(Value::Date)
                .ok_or_else(|| FormulaError::new("date is out of range", span))
        }
        other => Err(FormulaError::new(
            format!("dates move by whole days, not {}", other.type_name()),
            span,
        )),
    }
}
