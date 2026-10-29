// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Typed bodies of the data fences export needs for its fallbacks.
//!
//! The grammars are the cheat sheet's. `vellum_format::blocks` will own
//! them for every consumer (decision recorded with the coordinator); until
//! it lands they live here, behind this one module, so switching is a
//! change of imports. Parsing never fails: lines that do not match are
//! skipped, and a fence with nothing usable comes back empty so the
//! caller shows its source.

use vellum_format::Fence;

/// `key: value` lines of a fence body, in order. Keys are trimmed and
/// lower-cased; values are trimmed.
#[must_use]
pub fn pairs(literal: &str) -> Vec<(String, String)> {
    literal
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            (!key.is_empty()).then(|| (key.to_lowercase(), value.trim().to_owned()))
        })
        .collect()
}

/// The value of `key` in a fence's `key=value` arguments or attributes.
#[must_use]
pub fn arg(fence: &Fence, key: &str) -> Option<String> {
    if let Some((_, value)) = fence.attrs.pairs.iter().find(|(k, _)| k == key) {
        return Some(value.clone());
    }
    fence.args.split_whitespace().find_map(|word| {
        let (k, v) = word.split_once('=')?;
        (k == key).then(|| v.trim_matches('"').to_owned())
    })
}

/// Splits a `[a, b, c]` list (brackets optional) into trimmed items.
fn list(value: &str) -> Vec<String> {
    let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn number(text: &str) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '_' | ' ' | '%' | '$'))
        .collect();
    cleaned.parse().ok().filter(|n: &f64| n.is_finite())
}

/// A static chart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chart {
    /// Chart title; may be empty.
    pub title: String,
    /// `bar`, `line`, `donut` and so on.
    pub kind: String,
    /// Category labels (empty for donut and pie, whose slices are series).
    pub labels: Vec<String>,
    /// Named series of values; a single unnamed series is called `Value`.
    pub series: Vec<(String, Vec<f64>)>,
    /// Unit suffix; may be empty.
    pub unit: String,
}

/// Parses a `chart` fence.
#[must_use]
pub fn chart(fence: &Fence) -> Chart {
    let mut chart = Chart::default();
    let mut slices = Vec::new();
    for (key, value) in pairs(&fence.literal) {
        match key.as_str() {
            "title" => chart.title = value,
            "type" => chart.kind = value,
            "unit" => chart.unit = value,
            "labels" => chart.labels = list(&value),
            "values" => chart.series.push((
                "Value".to_owned(),
                list(&value).iter().filter_map(|v| number(v)).collect(),
            )),
            "ymin" | "ymax" | "legend" | "colors" | "source" | "x" | "y" => {}
            _ if value.starts_with('[') => chart.series.push((
                original_key(&fence.literal, &key),
                list(&value).iter().filter_map(|v| number(v)).collect(),
            )),
            _ => {
                if let Some(n) = number(&value) {
                    slices.push((original_key(&fence.literal, &key), n));
                }
            }
        }
    }
    if chart.series.is_empty() && !slices.is_empty() {
        chart.labels = slices.iter().map(|(label, _)| label.clone()).collect();
        chart
            .series
            .push(("Value".to_owned(), slices.into_iter().map(|(_, n)| n).collect()));
    }
    chart
}

/// The key of a `key: value` line as written, for series and slice names.
fn original_key(literal: &str, lower: &str) -> String {
    literal
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, _)| key.trim())
        .find(|key| key.to_lowercase() == lower)
        .unwrap_or(lower)
        .to_owned()
}

/// One event of a `timeline` fence: `when | title | detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The date as written.
    pub when: String,
    /// What happened.
    pub title: String,
    /// More detail; may be empty.
    pub detail: String,
}

fn cells(line: &str) -> Vec<String> {
    line.split('|').map(|cell| cell.trim().to_owned()).collect()
}

/// Parses a `timeline` fence.
#[must_use]
pub fn timeline(fence: &Fence) -> Vec<Event> {
    fence
        .literal
        .lines()
        .filter(|line| line.contains('|'))
        .map(|line| {
            let mut parts = cells(line).into_iter();
            Event {
                when: parts.next().unwrap_or_default(),
                title: parts.next().unwrap_or_default(),
                detail: parts.collect::<Vec<_>>().join(" | "),
            }
        })
        .collect()
}

/// One row of a `roadmap` fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    /// The work item.
    pub name: String,
    /// Start date, `YYYY-MM-DD`.
    pub start: String,
    /// End date; `None` for a milestone.
    pub end: Option<String>,
}

/// Parses a `roadmap` fence: `range:` then `name | start | end` rows, where
/// `end` may be `milestone`.
#[must_use]
pub fn roadmap(fence: &Fence) -> (Option<String>, Vec<Bar>) {
    let mut range = None;
    let mut bars = Vec::new();
    for line in fence.literal.lines() {
        if let Some(rest) = line.trim().strip_prefix("range:") {
            range = Some(rest.trim().to_owned());
        } else if line.contains('|') {
            let parts = cells(line);
            let name = parts.first().cloned().unwrap_or_default();
            let start = parts.get(1).cloned().unwrap_or_default();
            let end = parts
                .get(2)
                .filter(|end| !end.eq_ignore_ascii_case("milestone") && !end.is_empty())
                .cloned();
            bars.push(Bar { name, start, end });
        }
    }
    (range, bars)
}

/// One risk of a `risk-matrix` fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Risk {
    /// The risk.
    pub title: String,
    /// Likelihood, 1 to 5.
    pub likelihood: u8,
    /// Impact, 1 to 5.
    pub impact: u8,
}

/// Parses a `risk-matrix` fence: `title | likelihood N | impact M`.
#[must_use]
pub fn risks(fence: &Fence) -> Vec<Risk> {
    let score = |cell: &str, word: &str| -> Option<u8> {
        cell.trim()
            .strip_prefix(word)?
            .trim()
            .parse()
            .ok()
            .map(|n: u8| n.clamp(1, 5))
    };
    fence
        .literal
        .lines()
        .filter(|line| line.contains('|'))
        .map(|line| {
            let parts = cells(line);
            let find = |word: &str| parts.iter().find_map(|c| score(c, word)).unwrap_or(1);
            Risk {
                title: parts.first().cloned().unwrap_or_default(),
                likelihood: find("likelihood"),
                impact: find("impact"),
            }
        })
        .collect()
}

/// A column of a `board` fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The column name.
    pub name: String,
    /// Cards: text and an optional `[tag]`.
    pub cards: Vec<(String, Option<String>)>,
}

/// Parses a `board` fence: `## Column` headings and `- card [tag]` items.
#[must_use]
pub fn board(fence: &Fence) -> Vec<Column> {
    let mut columns: Vec<Column> = Vec::new();
    for line in fence.literal.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("## ") {
            columns.push(Column {
                name: name.trim().to_owned(),
                cards: Vec::new(),
            });
        } else if let Some(card) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            let card = card.trim();
            let (text, tag) = match card.strip_suffix(']').and_then(|c| c.rsplit_once('[')) {
                Some((text, tag)) => (text.trim().to_owned(), Some(tag.trim().to_owned())),
                None => (card.to_owned(), None),
            };
            if columns.is_empty() {
                columns.push(Column {
                    name: String::new(),
                    cards: Vec::new(),
                });
            }
            if let Some(column) = columns.last_mut() {
                column.cards.push((text, tag));
            }
        }
    }
    columns
}

/// A CSV `table` fence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsvTable {
    /// The header row.
    pub header: Vec<String>,
    /// Body rows, padded or cut to the header width, in export order.
    pub rows: Vec<Vec<String>>,
}

/// Parses a `table` fence. A `sort=Column` argument (with `desc` for
/// descending) gives the current sort order; numbers sort numerically.
#[must_use]
pub fn csv_table(fence: &Fence) -> CsvTable {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(fence.literal.as_bytes());
    let mut records = reader
        .records()
        .filter_map(Result::ok)
        .map(|record| record.iter().map(str::to_owned).collect::<Vec<_>>());
    let header = records.next().unwrap_or_default();
    let width = header.len();
    let mut rows: Vec<Vec<String>> = records
        .map(|mut row| {
            row.resize(width, String::new());
            row
        })
        .collect();
    if let Some(sort) = arg(fence, "sort") {
        let (name, descending) = match sort.split_once(':') {
            Some((name, dir)) => (name.to_owned(), dir.eq_ignore_ascii_case("desc")),
            None => (sort, fence.args.split_whitespace().any(|w| w == "desc")),
        };
        if let Some(index) = header.iter().position(|h| h == &name) {
            rows.sort_by(|a, b| {
                let (a, b) = (a.get(index), b.get(index));
                let order = match (a.and_then(|v| number(v)), b.and_then(|v| number(v))) {
                    (Some(x), Some(y)) => x.total_cmp(&y),
                    _ => a.cmp(&b),
                };
                if descending { order.reverse() } else { order }
            });
        }
    }
    CsvTable { header, rows }
}

/// An `architecture` fence: lanes of items and arrows between items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Architecture {
    /// `Lane: item, item` lines.
    pub lanes: Vec<(String, Vec<String>)>,
    /// `a -> b` lines.
    pub links: Vec<(String, String)>,
}

/// Parses an `architecture` fence.
#[must_use]
pub fn architecture(fence: &Fence) -> Architecture {
    let mut arch = Architecture::default();
    for line in fence.literal.lines() {
        if let Some((from, to)) = line.split_once("->") {
            arch.links
                .push((from.trim().to_owned(), to.trim().to_owned()));
        } else if let Some((lane, items)) = line.split_once(':') {
            arch.lanes.push((lane.trim().to_owned(), list(items)));
        }
    }
    arch
}

/// One slider of a `calculator` fence.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    /// The variable name used in formulas.
    pub key: String,
    /// The label shown to readers.
    pub label: String,
    /// The current (stored) value.
    pub value: f64,
}

/// One computed output of a `calculator` fence.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The label shown to readers.
    pub label: String,
    /// The computed value, or why it could not be computed.
    pub value: Result<f64, String>,
    /// Unit, such as `$`; may be empty.
    pub unit: String,
}

/// Parses a `calculator` fence and computes its outputs from the stored
/// input values:
///
/// ```text
/// units: Units = 100 [0..1000 step 10]
/// result: Revenue = units * price ($)
/// ```
#[must_use]
pub fn calculator(fence: &Fence) -> (Vec<Input>, Vec<Output>) {
    let mut inputs = Vec::new();
    let mut formulas = Vec::new();
    for line in fence.literal.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some((label, rhs)) = rest.split_once('=') else {
            continue;
        };
        let rhs = rhs.trim();
        let value_part = rhs.split('[').next().unwrap_or(rhs).trim();
        match number(value_part) {
            Some(value) if !rhs.contains('(') || rhs.contains('[') => inputs.push(Input {
                key: key.trim().to_owned(),
                label: label.trim().to_owned(),
                value,
            }),
            _ => {
                let (expr, unit) = match rhs.rsplit_once('(') {
                    Some((expr, unit)) if unit.ends_with(')') && !expr.trim().is_empty() => {
                        (expr.trim(), unit.trim_end_matches(')').trim())
                    }
                    _ => (rhs, ""),
                };
                formulas.push((label.trim().to_owned(), expr.to_owned(), unit.to_owned()));
            }
        }
    }
    let outputs = formulas
        .into_iter()
        .map(|(label, expr, unit)| Output {
            label,
            value: eval(&expr, &inputs),
            unit,
        })
        .collect();
    (inputs, outputs)
}

/// Evaluates a calculator formula: numbers, input names, `+ - * / %`,
/// parentheses and `round min max abs`.
///
/// # Errors
///
/// A message when the formula does not parse, names an unknown input or
/// divides by zero.
pub fn eval(expr: &str, inputs: &[Input]) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut parser = Calc {
        tokens: &tokens,
        pos: 0,
        inputs,
        depth: 0,
    };
    let value = parser.sum()?;
    if parser.pos != tokens.len() {
        return Err(format!("unexpected text in formula `{expr}`"));
    }
    if value.is_finite() {
        Ok(value)
    } else {
        Err("the result is not a finite number".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(char),
}

fn tokenize(expr: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = expr.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            let _ = chars.next();
        } else if c.is_ascii_digit() || c == '.' {
            let mut text = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() || d == '.' || d == '_' {
                    if d != '_' {
                        text.push(d);
                    }
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Num(
                text.parse().map_err(|_| format!("bad number `{text}`"))?,
            ));
        } else if c.is_alphabetic() || c == '_' {
            let mut text = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_alphanumeric() || d == '_' {
                    text.push(d);
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            tokens.push(Token::Name(text));
        } else if "+-*/%(),".contains(c) {
            tokens.push(Token::Op(c));
            let _ = chars.next();
        } else {
            return Err(format!("unexpected `{c}` in formula"));
        }
    }
    Ok(tokens)
}

struct Calc<'a> {
    tokens: &'a [Token],
    pos: usize,
    inputs: &'a [Input],
    depth: usize,
}

impl Calc<'_> {
    fn peek_op(&self) -> Option<char> {
        match self.tokens.get(self.pos) {
            Some(Token::Op(c)) => Some(*c),
            _ => None,
        }
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut value = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek_op() {
            self.pos += 1;
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        while let Some(op @ ('*' | '/' | '%')) = self.peek_op() {
            self.pos += 1;
            let rhs = self.unary()?;
            value = match op {
                '*' => value * rhs,
                _ if rhs == 0.0 => return Err("division by zero".to_owned()),
                '/' => value / rhs,
                _ => value % rhs,
            };
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.peek_op() == Some('-') {
            self.pos += 1;
            return Ok(-self.unary()?);
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<f64, String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("formula is nested too deeply".to_owned());
        }
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        let value = match token {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::Op('(')) => {
                let value = self.sum()?;
                self.expect(')')?;
                Ok(value)
            }
            Some(Token::Name(name)) if self.peek_op() == Some('(') => self.call(&name),
            Some(Token::Name(name)) => self
                .inputs
                .iter()
                .find(|input| input.key == name)
                .map(|input| input.value)
                .ok_or_else(|| format!("unknown input `{name}`")),
            _ => Err("incomplete formula".to_owned()),
        };
        self.depth -= 1;
        value
    }

    fn call(&mut self, name: &str) -> Result<f64, String> {
        self.expect('(')?;
        let mut args = vec![self.sum()?];
        while self.peek_op() == Some(',') {
            self.pos += 1;
            args.push(self.sum()?);
        }
        self.expect(')')?;
        let first = args.first().copied().unwrap_or_default();
        match name {
            "round" => {
                let places = args.get(1).copied().unwrap_or_default();
                let scale = 10f64.powf(places.clamp(0.0, 12.0).trunc());
                Ok((first * scale).round() / scale)
            }
            "abs" => Ok(first.abs()),
            "min" => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
            "max" => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            _ => Err(format!("unknown function `{name}`")),
        }
    }

    fn expect(&mut self, op: char) -> Result<(), String> {
        if self.peek_op() == Some(op) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected `{op}` in formula"))
        }
    }
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
    use vellum_format::Attributes;

    use super::*;

    fn fence(lang: &str, args: &str, literal: &str) -> Fence {
        Fence {
            lang: lang.into(),
            args: args.into(),
            attrs: Attributes::default(),
            literal: literal.into(),
        }
    }

    #[test]
    fn charts_read_single_grouped_and_donut_forms() {
        let bar = chart(&fence(
            "chart",
            "",
            "title: Blocked\ntype: bar\nlabels: [W40, W41]\nvalues: [120, 185]\n",
        ));
        assert_eq!(bar.title, "Blocked");
        assert_eq!(bar.labels, ["W40", "W41"]);
        assert_eq!(bar.series, [("Value".to_owned(), vec![120.0, 185.0])]);

        let grouped = chart(&fence(
            "chart",
            "",
            "type: grouped\nlabels: [X, Y]\nBefore: [4, 6]\nAfter: [7, 8]\n",
        ));
        assert_eq!(grouped.series.len(), 2);
        assert_eq!(grouped.series.first().map(|s| s.0.as_str()), Some("Before"));

        let donut = chart(&fence(
            "chart",
            "",
            "type: donut\nDirect: 60\nMulti-turn: 10\n",
        ));
        assert_eq!(donut.labels, ["Direct", "Multi-turn"]);
        assert_eq!(donut.series, [("Value".to_owned(), vec![60.0, 10.0])]);
    }

    #[test]
    fn timeline_roadmap_and_risks_parse_their_rows() {
        let events = timeline(&fence("timeline", "", "Oct 6 | Approved | Scope\nOct 20 | Staging\n"));
        assert_eq!(events.len(), 2);
        assert_eq!(events.get(1).map(|e| e.detail.as_str()), Some(""));

        let (range, bars) = roadmap(&fence(
            "roadmap",
            "",
            "range: 2026-11-01 .. 2027-06-30\nBuild | 2026-12-01 | 2027-04-30\nLaunch | 2027-05-15 | milestone\n",
        ));
        assert_eq!(range.as_deref(), Some("2026-11-01 .. 2027-06-30"));
        assert_eq!(bars.get(1).and_then(|b| b.end.clone()), None);

        let risks = risks(&fence("risk-matrix", "", "PDF slips | likelihood 3 | impact 9\n"));
        assert_eq!(
            risks,
            [Risk {
                title: "PDF slips".into(),
                likelihood: 3,
                impact: 5
            }]
        );
    }

    #[test]
    fn boards_keep_columns_and_tags() {
        let columns = board(&fence("board", "", "## To do\n- First [P1]\n## Done\n- Shipped\n"));
        assert_eq!(columns.len(), 2);
        assert_eq!(
            columns.first().map(|c| c.cards.clone()),
            Some(vec![("First".to_owned(), Some("P1".to_owned()))])
        );
    }

    #[test]
    fn csv_tables_sort_numerically_when_asked() {
        let table = csv_table(&fence(
            "table",
            "sortable sort=Value desc",
            "Name, Value\nA, 9\nB, 120\nC, 30\n",
        ));
        let names: Vec<_> = table.rows.iter().filter_map(|r| r.first().cloned()).collect();
        assert_eq!(names, ["B", "C", "A"]);
    }

    #[test]
    fn calculators_compute_outputs_from_stored_values() {
        let (inputs, outputs) = calculator(&fence(
            "calculator",
            "",
            "units: Units = 100 [0..1000 step 10]\nprice: Price = 20 [1..100 step 1]\nresult: Revenue = units * price ($)\n",
        ));
        assert_eq!(inputs.len(), 2);
        assert_eq!(outputs.first().map(|o| o.value.clone()), Some(Ok(2000.0)));
        assert_eq!(outputs.first().map(|o| o.unit.as_str()), Some("$"));
    }

    #[test]
    fn formulas_report_errors_instead_of_failing() {
        let inputs = [Input {
            key: "a".into(),
            label: "A".into(),
            value: 2.0,
        }];
        assert_eq!(eval("round(a / 3, 2)", &inputs), Ok(0.67));
        assert_eq!(eval("max(a, 5) - -1", &inputs), Ok(6.0));
        assert!(eval("a / 0", &inputs).is_err());
        assert!(eval("b + 1", &inputs).is_err());
        assert!(eval("(((a)", &inputs).is_err());
        assert!(eval(&"(".repeat(500), &inputs).is_err());
    }

    #[test]
    fn numbers_display_with_separators() {
        assert_eq!(display_number(1_234_567.0), "1,234,567");
        assert_eq!(display_number(-0.5), "-0.5");
        assert_eq!(display_number(12.345), "12.35");
    }
}
