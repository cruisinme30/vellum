// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The formula syntax tree and its canonical printer.

use std::fmt;

use serde::Serialize;

use crate::value::format_number;

/// A range of characters (Unicode scalar values, not bytes) in the source,
/// so editors can underline the exact spot of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
pub struct Span {
    /// First character, counting from 0.
    pub start: usize,
    /// One past the last character.
    pub end: usize,
}

impl Span {
    pub(crate) const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub(crate) fn to(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `not x`
    Not,
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+` (numbers add, text concatenates, date + days)
    Add,
    /// `-` (numbers, date − days, date − date in days)
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `and`
    And,
    /// `or`
    Or,
}

impl BinOp {
    /// Binding strength; higher binds tighter.
    pub(crate) const fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Eq | Self::Ne | Self::Lt | Self::Gt | Self::Le | Self::Ge => 4,
            Self::Add | Self::Sub => 5,
            Self::Mul | Self::Div | Self::Rem => 6,
        }
    }

    const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::And => "and",
            Self::Or => "or",
        }
    }
}

/// A built-in function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Func {
    /// `if(cond, then, else?)`
    If,
    /// `round(x, digits?)`
    Round,
    /// `len(text | list)`
    Len,
    /// `concat(a, b, …)`
    Concat,
    /// `min(…)`
    Min,
    /// `max(…)`
    Max,
    /// `sum(…)`
    Sum,
    /// `avg(…)`
    Avg,
    /// `count(…)`
    Count,
    /// `now()`
    Now,
    /// `year(date)`
    Year,
    /// `days(a, b)`: whole days from `b` to `a`
    Days,
    /// `format(x)`
    Format,
    /// `percent(x, digits?)`
    Percent,
    /// `empty(x)`
    Empty,
    /// `contains(text | list, x)`
    Contains,
    /// `upper(text)`
    Upper,
    /// `lower(text)`
    Lower,
}

impl Func {
    /// Every function, in documentation order.
    pub const ALL: [Self; 18] = [
        Self::If,
        Self::Round,
        Self::Len,
        Self::Concat,
        Self::Min,
        Self::Max,
        Self::Sum,
        Self::Avg,
        Self::Count,
        Self::Now,
        Self::Year,
        Self::Days,
        Self::Format,
        Self::Percent,
        Self::Empty,
        Self::Contains,
        Self::Upper,
        Self::Lower,
    ];

    /// The function's name in formulas.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::If => "if",
            Self::Round => "round",
            Self::Len => "len",
            Self::Concat => "concat",
            Self::Min => "min",
            Self::Max => "max",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Count => "count",
            Self::Now => "now",
            Self::Year => "year",
            Self::Days => "days",
            Self::Format => "format",
            Self::Percent => "percent",
            Self::Empty => "empty",
            Self::Contains => "contains",
            Self::Upper => "upper",
            Self::Lower => "lower",
        }
    }

    /// Finds a function by name, ignoring case.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
    }

    /// Smallest and largest argument counts (`None` = unlimited).
    #[must_use]
    pub const fn arity(self) -> (usize, Option<usize>) {
        match self {
            Self::Now => (0, Some(0)),
            Self::Len | Self::Year | Self::Format | Self::Empty | Self::Upper | Self::Lower => {
                (1, Some(1))
            }
            Self::Round | Self::Percent => (1, Some(2)),
            Self::Days | Self::Contains => (2, Some(2)),
            Self::If => (2, Some(3)),
            Self::Concat | Self::Min | Self::Max | Self::Sum | Self::Avg | Self::Count => (1, None),
        }
    }
}

/// An expression with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The expression.
    pub expr: Expr,
    /// Where it came from.
    pub span: Span,
}

/// A formula expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number literal.
    Number(f64),
    /// A text literal.
    Text(String),
    /// `true` or `false`.
    Bool(bool),
    /// A property reference: the property name as written until compiled,
    /// then the property key.
    Prop(String),
    /// A unary operation.
    Unary(UnOp, Box<Node>),
    /// A binary operation.
    Binary(BinOp, Box<Node>, Box<Node>),
    /// A function call.
    Call(Func, Vec<Node>),
}

impl Node {
    pub(crate) const fn new(expr: Expr, span: Span) -> Self {
        Self { expr, span }
    }

    pub(crate) fn walk_mut(&mut self, f: &mut impl FnMut(&mut Self)) {
        f(self);
        match &mut self.expr {
            Expr::Unary(_, e) => e.walk_mut(f),
            Expr::Binary(_, l, r) => {
                l.walk_mut(f);
                r.walk_mut(f);
            }
            Expr::Call(_, args) => args.iter_mut().for_each(|a| a.walk_mut(f)),
            Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Prop(_) => {}
        }
    }

    const fn precedence(&self) -> u8 {
        match &self.expr {
            Expr::Binary(op, ..) => op.precedence(),
            Expr::Unary(UnOp::Not, _) => 3,
            Expr::Unary(UnOp::Neg, _) => 7,
            _ => 8,
        }
    }
}

/// Prints the canonical form: fully explicit `prop("…")` references,
/// minimal parentheses, spaces around binary operators.
impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.expr {
            Expr::Number(n) => f.write_str(&format_number(*n)),
            Expr::Text(t) => write_string(f, t),
            Expr::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Expr::Prop(name) => {
                f.write_str("prop(")?;
                write_string(f, name)?;
                f.write_str(")")
            }
            Expr::Unary(op, e) => {
                let own = self.precedence();
                f.write_str(match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "not ",
                })?;
                // `- -x` must not print as `--x`; keep nested unary in parens.
                let wrap = e.precedence() < own
                    || matches!(e.expr, Expr::Unary(..))
                    || matches!(e.expr, Expr::Number(n) if n < 0.0);
                write_child(f, e, wrap)
            }
            Expr::Binary(op, l, r) => {
                let own = op.precedence();
                // Left-associative: the right child needs parens at equal
                // precedence, the left one only below it.
                write_child(f, l, l.precedence() < own)?;
                write!(f, " {} ", op.symbol())?;
                write_child(f, r, r.precedence() <= own)
            }
            Expr::Call(func, args) => {
                write!(f, "{}(", func.name())?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    fmt::Display::fmt(a, f)?;
                }
                f.write_str(")")
            }
        }
    }
}

fn write_child(f: &mut fmt::Formatter<'_>, node: &Node, wrap: bool) -> fmt::Result {
    if wrap {
        write!(f, "({node})")
    } else {
        fmt::Display::fmt(node, f)
    }
}

fn write_string(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in text.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\t' => f.write_str("\\t")?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}
