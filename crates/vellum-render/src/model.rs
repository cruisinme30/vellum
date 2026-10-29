// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Drawing views of the typed block data from [`vellum_format::blocks`]:
//! dates as day numbers, defaults filled in, and the safe formula evaluator.

use vellum_format::blocks as data;

pub use vellum_format::blocks::{ChartKind, Risk};

/// Reads `1,234.5`, `40%`, `$12` as numbers.
#[must_use]
pub fn parse_number(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '_' | '%' | '$' | '€' | '£' | ' ' | '\u{a0}'))
        .collect();
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// One data series.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// Legend name.
    pub name: String,
    /// One value per label (or per point for scatter).
    pub values: Vec<f64>,
    /// X values for scatter charts.
    pub xs: Vec<f64>,
}

/// A chart, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// Title, shown as the caption.
    pub title: String,
    /// Kind.
    pub kind: ChartKind,
    /// Category labels.
    pub labels: Vec<String>,
    /// Series; pie and donut use the first.
    pub series: Vec<Series>,
    /// Unit suffix for values.
    pub unit: String,
    /// Fixed axis minimum.
    pub ymin: Option<f64>,
    /// Fixed axis maximum.
    pub ymax: Option<f64>,
    /// Whether to show a legend.
    pub legend: bool,
}

/// The drawing view of a `chart` fence. Scatter charts read their labels as
/// x values.
///
/// # Errors
///
/// Returns a message when the chart has no values.
pub fn chart(c: &data::Chart) -> Result<Chart, String> {
    let xs: Vec<f64> = if c.kind == ChartKind::Scatter {
        c.labels.iter().filter_map(|l| parse_number(l)).collect()
    } else {
        Vec::new()
    };
    let series: Vec<Series> = c
        .series
        .iter()
        .filter(|s| !s.values.is_empty())
        .map(|s| Series {
            name: s.name.clone(),
            values: s.values.clone(),
            xs: xs.clone(),
        })
        .collect();
    if series.is_empty() {
        return Err("chart has no values".to_owned());
    }
    let named = series.iter().filter(|s| !s.name.is_empty()).count();
    Ok(Chart {
        title: c.title.clone(),
        kind: c.kind,
        labels: c.labels.clone(),
        unit: c.unit.clone().unwrap_or_default(),
        ymin: c.ymin,
        ymax: c.ymax,
        legend: c.legend.unwrap_or(named > 1),
        series,
    })
}

/// A calendar date as days since 1970-01-01.
#[must_use]
pub fn days_from_iso(s: &str) -> Option<i64> {
    let mut parts = s.trim().splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `(year, month)` of a day number.
#[must_use]
pub const fn year_month(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m)
}

/// One roadmap row as day numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoadmapRow {
    /// Name.
    pub name: String,
    /// Start day.
    pub start: i64,
    /// End day (equal to start for milestones).
    pub end: i64,
    /// True for a milestone diamond.
    pub milestone: bool,
}

/// A roadmap as day numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roadmap {
    /// First day shown.
    pub start: i64,
    /// Last day shown.
    pub end: i64,
    /// Rows, in order.
    pub rows: Vec<RoadmapRow>,
}

/// The drawing view of a `roadmap` fence.
///
/// # Errors
///
/// Returns a message when no row has valid dates.
pub fn roadmap(r: &data::Roadmap) -> Result<Roadmap, String> {
    let rows: Vec<RoadmapRow> = r
        .rows
        .iter()
        .filter_map(|row| {
            let start = days_from_iso(&row.start)?;
            Some(match row.end.as_deref().and_then(days_from_iso) {
                Some(end) => RoadmapRow {
                    name: row.name.clone(),
                    start: start.min(end),
                    end: start.max(end),
                    milestone: false,
                },
                None => RoadmapRow {
                    name: row.name.clone(),
                    start,
                    end: start,
                    milestone: true,
                },
            })
        })
        .collect();
    if rows.is_empty() {
        return Err("roadmap has no rows".to_owned());
    }
    let start = r
        .start
        .as_deref()
        .and_then(days_from_iso)
        .unwrap_or_else(|| rows.iter().map(|r| r.start).min().unwrap_or(0));
    let end = r
        .end
        .as_deref()
        .and_then(days_from_iso)
        .unwrap_or_else(|| rows.iter().map(|r| r.end).max().unwrap_or(start));
    Ok(Roadmap {
        start,
        end: end.max(start + 1),
        rows,
    })
}

/// Evaluates `+ - * / % ( )` over numbers and named inputs.
///
/// # Errors
///
/// Returns a message for unknown names, bad syntax or division by zero.
pub fn evaluate(formula: &str, vars: &[(String, f64)]) -> Result<f64, String> {
    let tokens: Vec<char> = formula.chars().filter(|c| !c.is_whitespace()).collect();
    let mut p = Eval {
        tokens: &tokens,
        pos: 0,
        vars,
        depth: 0,
    };
    let v = p.expr()?;
    if p.pos != tokens.len() {
        return Err(format!(
            "unexpected `{}`",
            tokens.get(p.pos).copied().unwrap_or(' ')
        ));
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err("result is not a number".to_owned())
    }
}

struct Eval<'a> {
    tokens: &'a [char],
    pos: usize,
    vars: &'a [(String, f64)],
    depth: usize,
}

impl Eval<'_> {
    fn peek(&self) -> Option<char> {
        self.tokens.get(self.pos).copied()
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.pos += 1;
            let r = self.term()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        while let Some(op @ ('*' | '/' | '%' | '×' | '÷')) = self.peek() {
            self.pos += 1;
            let r = self.factor()?;
            v = match op {
                '*' | '×' => v * r,
                _ if r == 0.0 => return Err("division by zero".to_owned()),
                '%' => v % r,
                _ => v / r,
            };
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<f64, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("formula is nested too deeply".to_owned());
        }
        let result = match self.peek() {
            Some('-') => {
                self.pos += 1;
                self.factor().map(|v| -v)
            }
            Some('(') => {
                self.pos += 1;
                let v = self.expr()?;
                if self.peek() == Some(')') {
                    self.pos += 1;
                    Ok(v)
                } else {
                    Err("missing `)`".to_owned())
                }
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    self.pos += 1;
                }
                let s: String = self
                    .tokens
                    .get(start..self.pos)
                    .unwrap_or_default()
                    .iter()
                    .collect();
                s.parse().map_err(|_| format!("bad number `{s}`"))
            }
            Some(c) if c.is_alphabetic() || c == '_' => {
                let start = self.pos;
                while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    self.pos += 1;
                }
                let name: String = self
                    .tokens
                    .get(start..self.pos)
                    .unwrap_or_default()
                    .iter()
                    .collect();
                self.vars
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| format!("unknown input `{name}`"))
            }
            _ => Err("formula ends early".to_owned()),
        };
        self.depth -= 1;
        result
    }
}

/// Reads the typed data of a `lang` fence with `body`, for tests.
#[cfg(test)]
#[must_use]
#[allow(clippy::missing_panics_doc, reason = "test helper")]
pub fn fence_data(lang: &str, body: &str) -> data::Data {
    let doc = vellum_format::parse(&format!("```{lang}\n{body}```\n")).document;
    let block = doc.blocks.first().expect("one block");
    data::Data::from_block(block).expect("a data fence").data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(src: &str) -> data::Data {
        let src = src.strip_prefix("```").unwrap();
        let (lang, rest) = src.split_once('\n').unwrap();
        fence_data(lang, rest.strip_suffix("```\n").unwrap())
    }

    #[test]
    fn chart_views() {
        let data::Data::Chart(c) = read(
            "```chart\ntitle: T\ntype: grouped\nlabels: [X, Y]\nBefore: [4, 6]\nAfter: [7, 8]\n```\n",
        ) else {
            panic!()
        };
        let v = chart(&c).unwrap();
        assert!(v.legend);
        assert_eq!(v.series.len(), 2);
        let data::Data::Chart(s) =
            read("```chart\ntype: scatter\nlabels: [1, 2.5]\nvalues: [3, 4]\n```\n")
        else {
            panic!()
        };
        assert_eq!(chart(&s).unwrap().series[0].xs, [1.0, 2.5]);
        let data::Data::Chart(e) = read("```chart\ntitle: x\n```\n") else {
            panic!()
        };
        assert!(chart(&e).is_err());
    }

    #[test]
    fn roadmap_view() {
        let data::Data::Roadmap(r) = read(
            "```roadmap\nrange: 2026-11-01 .. 2027-06-30\nDiscovery | 2026-11-01 | 2026-12-15\nLaunch | 2027-05-15 | milestone\n```\n",
        ) else {
            panic!()
        };
        let v = roadmap(&r).unwrap();
        assert_eq!(v.rows.len(), 2);
        assert!(v.rows[1].milestone);
        assert_eq!(v.start, days_from_iso("2026-11-01").unwrap());
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        let d = days_from_iso("2027-03-15").unwrap();
        assert_eq!(year_month(d), (2027, 3));
        assert_eq!(days_from_iso("2027-13-01"), None);
    }

    #[test]
    fn evaluates_formulas() {
        let vars = [("units".to_owned(), 100.0), ("price".to_owned(), 20.0)];
        assert_eq!(evaluate("units * price", &vars), Ok(2000.0));
        assert_eq!(evaluate("(1 + 2) * -3 % 4", &[]), Ok(-1.0));
        assert!(evaluate("1 / 0", &[]).is_err());
        assert!(evaluate("alert(1)", &[]).is_err());
        assert!(evaluate(&"(".repeat(500), &[]).is_err());
        assert_eq!(parse_number("$1,234.5"), Some(1234.5));
    }
}
