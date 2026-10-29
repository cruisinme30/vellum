// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Data fences: one line-oriented format each, as in the cheat sheet.

use serde::Serialize;

use super::{Problems, Read, lines, list, number};
use crate::ast::Fence;

/// The chart types of dialect v1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    /// `bar` (the default)
    #[default]
    Bar,
    /// `line`
    Line,
    /// `area`
    Area,
    /// `donut`
    Donut,
    /// `pie`
    Pie,
    /// `grouped` bars
    Grouped,
    /// `stacked` bars
    Stacked,
    /// `scatter`; labels are x values
    Scatter,
}

impl ChartKind {
    fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "bar" => Self::Bar,
            "line" => Self::Line,
            "area" => Self::Area,
            "donut" => Self::Donut,
            "pie" => Self::Pie,
            "grouped" => Self::Grouped,
            "stacked" => Self::Stacked,
            "scatter" => Self::Scatter,
            _ => return None,
        })
    }
}

/// One named series of values, aligned with the chart labels.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Series {
    /// Series name; empty for the single `values:` series.
    pub name: String,
    /// One value per label.
    pub values: Vec<f64>,
}

/// A `chart` fence.
///
/// ```text
/// title: Blocked requests per week
/// type: bar
/// labels: [W40, W41, W42]
/// values: [120, 185, 160]        or one `Name: [..]` line per series,
///                                or `Slice: 60` lines for donut and pie
/// unit: ms   ymin: 0   ymax: 100   legend: true   colors: [#4F5B92]
/// ```
///
/// A live chart takes `{source=collection:<id> x=<prop> y=<prop>}`
/// attributes and no body.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Chart {
    /// Title shown above the chart.
    pub title: String,
    /// Chart type.
    pub kind: ChartKind,
    /// Category labels (x values for scatter).
    pub labels: Vec<String>,
    /// Data series.
    pub series: Vec<Series>,
    /// Unit appended to values.
    pub unit: Option<String>,
    /// Lower bound of the value axis.
    pub ymin: Option<f64>,
    /// Upper bound of the value axis.
    pub ymax: Option<f64>,
    /// Whether to show a legend; renderers decide when unset.
    pub legend: Option<bool>,
    /// Colours, in series order.
    pub colors: Vec<String>,
    /// Live data source, `collection:<id>`.
    pub source: Option<String>,
    /// Property for the x axis of a live chart.
    pub x: Option<String>,
    /// Property for the y axis of a live chart.
    pub y: Option<String>,
}

impl Chart {
    /// Reads a `chart` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut chart = Self::default();
        for (key, value) in &fence.attrs.pairs {
            match key.as_str() {
                "source" => chart.source = Some(value.clone()),
                "x" => chart.x = Some(value.clone()),
                "y" => chart.y = Some(value.clone()),
                _ => {}
            }
        }
        let mut slices: Vec<(String, f64)> = Vec::new();
        for (n, line) in lines(fence) {
            let Some((key, value)) = line.split_once(':') else {
                problems.add(n, format!("expected `key: value`, found `{line}`"));
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "title" => value.clone_into(&mut chart.title),
                "type" => match ChartKind::from_word(value) {
                    Some(kind) => chart.kind = kind,
                    None => problems.add(n, format!("unknown chart type `{value}`; using bar")),
                },
                "labels" => chart.labels = list(value),
                "unit" => chart.unit = Some(value.to_owned()),
                "ymin" => chart.ymin = number(value),
                "ymax" => chart.ymax = number(value),
                "legend" => chart.legend = Some(matches!(value, "true" | "yes" | "on")),
                "colors" | "colours" => chart.colors = list(value),
                "source" => chart.source = Some(value.to_owned()),
                key if value.starts_with('[') => {
                    let name = if key == "values" {
                        String::new()
                    } else {
                        key.to_owned()
                    };
                    let values = numbers(&list(value), n, &mut problems);
                    chart.series.push(Series { name, values });
                }
                key => match number(value) {
                    Some(v) => slices.push((key.to_owned(), v)),
                    None => problems.add(n, format!("`{key}` needs a number or a [list]")),
                },
            }
        }
        if chart.series.is_empty() && !slices.is_empty() {
            let (labels, values) = slices.into_iter().unzip();
            chart.labels = labels;
            chart.series.push(Series {
                name: String::new(),
                values,
            });
        }
        for series in &chart.series {
            if !chart.labels.is_empty() && series.values.len() != chart.labels.len() {
                problems.add(
                    0,
                    format!(
                        "series `{}` has {} values for {} labels",
                        series.name,
                        series.values.len(),
                        chart.labels.len()
                    ),
                );
            }
        }
        problems.done(chart)
    }
}

fn numbers(items: &[String], line: usize, problems: &mut Problems) -> Vec<f64> {
    items
        .iter()
        .filter_map(|item| {
            let value = number(item);
            if value.is_none() {
                problems.add(line, format!("`{item}` is not a number"));
            }
            value
        })
        .collect()
}

/// Splits `a | b | c` into trimmed cells.
fn cells(line: &str) -> Vec<&str> {
    line.split('|').map(str::trim).collect()
}

/// One `timeline` entry: `when | title | detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineItem {
    /// Date or period as written (`Oct 6`, `2026-11-24`, `Q1`).
    pub when: String,
    /// What happens.
    pub title: String,
    /// Optional detail.
    pub detail: Option<String>,
}

/// A `timeline` fence: one `when | title | detail` line per entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Timeline {
    /// Entries in source order.
    pub items: Vec<TimelineItem>,
}

impl Timeline {
    /// Reads a `timeline` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut timeline = Self::default();
        for (n, line) in lines(fence) {
            match cells(line).as_slice() {
                [when, title] => timeline.items.push(TimelineItem {
                    when: (*when).to_owned(),
                    title: (*title).to_owned(),
                    detail: None,
                }),
                [when, title, detail @ ..] => timeline.items.push(TimelineItem {
                    when: (*when).to_owned(),
                    title: (*title).to_owned(),
                    detail: Some(detail.join(" | ")).filter(|d| !d.is_empty()),
                }),
                _ => problems.add(n, format!("expected `when | title`, found `{line}`")),
            }
        }
        problems.done(timeline)
    }
}

/// One `roadmap` row: a bar from `start` to `end`, or a milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoadmapRow {
    /// Row label.
    pub name: String,
    /// Start date (`YYYY-MM-DD`), or the milestone date.
    pub start: String,
    /// End date; `None` for a milestone.
    pub end: Option<String>,
}

/// A `roadmap` fence.
///
/// ```text
/// range: 2026-11-01 .. 2027-06-30
/// Discovery | 2026-11-01 | 2026-12-15
/// Launch | 2027-05-15 | milestone
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Roadmap {
    /// First day shown.
    pub start: Option<String>,
    /// Last day shown.
    pub end: Option<String>,
    /// Rows in source order.
    pub rows: Vec<RoadmapRow>,
}

impl Roadmap {
    /// Reads a `roadmap` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut roadmap = Self::default();
        for (n, line) in lines(fence) {
            if let Some(range) = line.strip_prefix("range:") {
                match range.split_once("..") {
                    Some((a, b)) => {
                        roadmap.start = date(a.trim(), n, &mut problems);
                        roadmap.end = date(b.trim(), n, &mut problems);
                    }
                    None => problems.add(n, "expected `range: <start> .. <end>`"),
                }
                continue;
            }
            match cells(line).as_slice() {
                [name, start, "milestone"] | [name, start] => {
                    if let Some(start) = date(start, n, &mut problems) {
                        roadmap.rows.push(RoadmapRow {
                            name: (*name).to_owned(),
                            start,
                            end: None,
                        });
                    }
                }
                [name, start, end] => {
                    if let (Some(start), Some(end)) =
                        (date(start, n, &mut problems), date(end, n, &mut problems))
                    {
                        roadmap.rows.push(RoadmapRow {
                            name: (*name).to_owned(),
                            start,
                            end: Some(end),
                        });
                    }
                }
                _ => problems.add(n, format!("expected `name | start | end`, found `{line}`")),
            }
        }
        problems.done(roadmap)
    }
}

fn date(s: &str, line: usize, problems: &mut Problems) -> Option<String> {
    if crate::extras::is_date(s) {
        Some(s.to_owned())
    } else {
        problems.add(line, format!("`{s}` is not a YYYY-MM-DD date"));
        None
    }
}

/// One risk on the matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Risk {
    /// What could go wrong.
    pub title: String,
    /// 1 (rare) to 5 (almost certain).
    pub likelihood: u8,
    /// 1 (minor) to 5 (severe).
    pub impact: u8,
}

/// A `risk-matrix` fence: `title | likelihood 3 | impact 4` per line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RiskMatrix {
    /// Risks in source order.
    pub risks: Vec<Risk>,
}

impl RiskMatrix {
    /// Reads a `risk-matrix` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut matrix = Self::default();
        for (n, line) in lines(fence) {
            let parts = cells(line);
            let (mut likelihood, mut impact) = (None, None);
            for part in parts.iter().skip(1) {
                let (word, value) = part.split_once(' ').unwrap_or((part, ""));
                let score = value
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|v| (1..=5).contains(v));
                match word.to_ascii_lowercase().as_str() {
                    "likelihood" => likelihood = score,
                    "impact" => impact = score,
                    _ => {}
                }
            }
            match (parts.first(), likelihood, impact) {
                (Some(title), Some(likelihood), Some(impact)) if !title.is_empty() => {
                    matrix.risks.push(Risk {
                        title: (*title).to_owned(),
                        likelihood,
                        impact,
                    });
                }
                _ => problems.add(n, "expected `title | likelihood 1-5 | impact 1-5`"),
            }
        }
        problems.done(matrix)
    }
}

/// A card on a static board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardCard {
    /// Card text.
    pub text: String,
    /// Trailing `[TAG]`, such as a priority.
    pub tag: Option<String>,
}

/// A board column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardColumn {
    /// Column title.
    pub name: String,
    /// Cards, top to bottom.
    pub cards: Vec<BoardCard>,
}

/// A `board` fence: `## Column` lines followed by `- card [TAG]` lines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Board {
    /// Columns, left to right.
    pub columns: Vec<BoardColumn>,
}

impl Board {
    /// Reads a `board` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut board = Self::default();
        for (n, line) in lines(fence) {
            if let Some(name) = line.strip_prefix("## ") {
                board.columns.push(BoardColumn {
                    name: name.trim().to_owned(),
                    cards: Vec::new(),
                });
            } else if let Some(card) = line.strip_prefix("- ") {
                let card = card.trim();
                let (text, tag) = match card.strip_suffix(']').and_then(|c| c.rsplit_once('[')) {
                    Some((text, tag)) if !text.trim().is_empty() => {
                        (text.trim(), Some(tag.trim().to_owned()))
                    }
                    _ => (card, None),
                };
                match board.columns.last_mut() {
                    Some(column) => column.cards.push(BoardCard {
                        text: text.to_owned(),
                        tag,
                    }),
                    None => problems.add(n, "a card needs a `## Column` line above it"),
                }
            } else {
                problems.add(
                    n,
                    format!("expected `## Column` or `- card`, found `{line}`"),
                );
            }
        }
        problems.done(board)
    }
}

/// A `table` fence: CSV with a header row, and options on the info line
/// (`sortable`, `bar=<column>`, `sum=<column>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DataTable {
    /// Readers can sort by clicking a header.
    pub sortable: bool,
    /// Column drawn as inline bars.
    pub bar: Option<String>,
    /// Columns totalled in a footer row.
    pub sum: Vec<String>,
    /// Header cells.
    pub header: Vec<String>,
    /// Body rows; each has as many cells as the header.
    pub rows: Vec<Vec<String>>,
}

impl DataTable {
    /// Reads a `table` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut table = Self::default();
        let options = fence.args.split_whitespace().map(|w| (w, ""));
        let pairs = fence
            .attrs
            .pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()));
        for (word, value) in options.chain(pairs) {
            let (key, value) = word.split_once('=').unwrap_or((word, value));
            match key {
                "sortable" => table.sortable = true,
                "bar" => table.bar = Some(value.to_owned()),
                "sum" => table.sum.extend(value.split(',').map(str::to_owned)),
                _ => {}
            }
        }
        let mut rows = lines(fence).map(|(n, line)| (n, csv_row(line)));
        if let Some((_, header)) = rows.next() {
            table.header = header;
        }
        for (n, mut row) in rows {
            if row.len() != table.header.len() {
                problems.add(
                    n,
                    format!("{} cells for {} columns", row.len(), table.header.len()),
                );
                row.resize(table.header.len(), String::new());
            }
            table.rows.push(row);
        }
        problems.done(table)
    }
}

/// Splits one CSV line; `"…"` quotes a cell and `""` is a literal quote.
fn csv_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                let _ = chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            c => cell.push(c),
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}

/// A swim lane of an architecture diagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lane {
    /// Lane title.
    pub name: String,
    /// Boxes in the lane.
    pub items: Vec<String>,
}

/// An arrow between two boxes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Arrow {
    /// Box the arrow starts at.
    pub from: String,
    /// Box the arrow points to.
    pub to: String,
    /// Optional label (`A -> B: label`).
    pub label: Option<String>,
}

/// An `architecture` fence: `Lane: a, b` lines and `a -> b` arrows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Architecture {
    /// Lanes, top to bottom.
    pub lanes: Vec<Lane>,
    /// Arrows in source order.
    pub arrows: Vec<Arrow>,
}

impl Architecture {
    /// Reads an `architecture` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut arch = Self::default();
        for (n, line) in lines(fence) {
            if let Some((from, rest)) = line.split_once("->") {
                let (to, label) = match rest.split_once(':') {
                    Some((to, label)) => (to, Some(label.trim().to_owned())),
                    None => (rest, None),
                };
                arch.arrows.push(Arrow {
                    from: from.trim().to_owned(),
                    to: to.trim().to_owned(),
                    label,
                });
            } else if let Some((name, items)) = line.split_once(':') {
                arch.lanes.push(Lane {
                    name: name.trim().to_owned(),
                    items: list(items),
                });
            } else {
                problems.add(
                    n,
                    format!("expected `Lane: a, b` or `a -> b`, found `{line}`"),
                );
            }
        }
        let known: Vec<&String> = arch.lanes.iter().flat_map(|l| &l.items).collect();
        for arrow in &arch.arrows {
            for end in [&arrow.from, &arrow.to] {
                if !known.contains(&end) {
                    problems.add(0, format!("`{end}` is not in any lane"));
                }
            }
        }
        problems.done(arch)
    }
}

/// A slider input of a calculator.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalculatorInput {
    /// Name used in formulas.
    pub key: String,
    /// Label shown to readers.
    pub label: String,
    /// Starting value.
    pub value: f64,
    /// Slider minimum.
    pub min: Option<f64>,
    /// Slider maximum.
    pub max: Option<f64>,
    /// Slider step.
    pub step: Option<f64>,
}

/// A computed output of a calculator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalculatorOutput {
    /// Name usable in later formulas.
    pub key: String,
    /// Label shown to readers.
    pub label: String,
    /// Formula in the collections formula language.
    pub formula: String,
    /// Unit shown after the value.
    pub unit: Option<String>,
}

/// A `calculator` fence.
///
/// ```text
/// units: Units = 100 [0..1000 step 10]
/// result: Revenue = units * price ($)
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Calculator {
    /// Inputs (lines whose value is a number).
    pub inputs: Vec<CalculatorInput>,
    /// Outputs (lines whose value is a formula).
    pub outputs: Vec<CalculatorOutput>,
}

impl Calculator {
    /// Reads a `calculator` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut calc = Self::default();
        for (n, line) in lines(fence) {
            let Some((key, rest)) = line.split_once(':') else {
                problems.add(n, format!("expected `key: Label = value`, found `{line}`"));
                continue;
            };
            let (label, rhs) = rest.split_once('=').unwrap_or(("", rest));
            let (key, label, rhs) = (key.trim().to_owned(), label.trim(), rhs.trim());
            let label = if label.is_empty() {
                key.clone()
            } else {
                label.to_owned()
            };
            let (value, range) = match rhs.split_once('[') {
                Some((v, r)) => (v.trim(), r.strip_suffix(']').map(str::trim)),
                None => (rhs, None),
            };
            if let Some(value) = number(value) {
                let mut input = CalculatorInput {
                    key,
                    label,
                    value,
                    min: None,
                    max: None,
                    step: None,
                };
                if let Some(range) = range {
                    let (bounds, step) = range.split_once("step").unwrap_or((range, ""));
                    if let Some((min, max)) = bounds.split_once("..") {
                        input.min = number(min);
                        input.max = number(max);
                    }
                    input.step = number(step);
                    if input.min.is_none() || input.max.is_none() {
                        problems.add(n, "expected `[min..max step s]`");
                    }
                }
                calc.inputs.push(input);
            } else {
                let (formula, unit) = split_unit(rhs);
                calc.outputs.push(CalculatorOutput {
                    key,
                    label,
                    formula: formula.to_owned(),
                    unit,
                });
            }
        }
        problems.done(calc)
    }
}

/// Splits a trailing ` (unit)` off a formula. A parenthesis that is part
/// of the formula (`a * (b + c)`, `round(x)`) is not a unit: a unit
/// follows whitespace, and the formula before it must not end in an
/// operator.
fn split_unit(rhs: &str) -> (&str, Option<String>) {
    let Some((formula, unit)) = rhs.strip_suffix(')').and_then(|r| r.rsplit_once('(')) else {
        return (rhs, None);
    };
    let trimmed = formula.trim_end();
    let is_unit = !trimmed.is_empty()
        && formula.ends_with(char::is_whitespace)
        && !trimmed.ends_with([
            '+', '-', '*', '/', '^', '%', '(', ',', '=', '<', '>', '&', '|', '!',
        ])
        && !unit.trim().is_empty();
    if is_unit {
        (trimmed, Some(unit.trim().to_owned()))
    } else {
        (rhs, None)
    }
}

/// Reads `key: value` lines into `set`, reporting unknown keys.
fn key_values(fence: &Fence, problems: &mut Problems, mut set: impl FnMut(&str, String) -> bool) {
    for (n, line) in lines(fence) {
        match line.split_once(':') {
            Some((key, value)) => {
                if !set(key.trim(), value.trim().to_owned()) {
                    problems.add(n, format!("unknown key `{}`", key.trim()));
                }
            }
            None => problems.add(n, format!("expected `key: value`, found `{line}`")),
        }
    }
}

/// An `image` fence: `src`, `alt`, `caption`, `width`, `align` keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Image {
    /// Image path or URL.
    pub src: String,
    /// Alternative text.
    pub alt: String,
    /// Caption shown below.
    pub caption: Option<String>,
    /// Width as written (`480`, `50%`).
    pub width: Option<String>,
    /// `left`, `center` or `right`.
    pub align: Option<String>,
}

impl Image {
    /// Reads an `image` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut image = Self::default();
        key_values(fence, &mut problems, |key, value| {
            match key {
                "src" => image.src = value,
                "alt" => image.alt = value,
                "caption" => image.caption = Some(value),
                "width" => image.width = Some(value),
                "align" => image.align = Some(value),
                _ => return false,
            }
            true
        });
        if image.src.is_empty() {
            problems.add(0, "an image needs `src:`");
        }
        problems.done(image)
    }
}

/// A `collection` fence: a view of a collection embedded in a page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CollectionView {
    /// Collection ID (`id:`, or `source: collection:<id>`).
    pub id: String,
    /// Saved view name.
    pub view: Option<String>,
    /// Extra filter, in the views filter syntax.
    pub filter: Option<String>,
}

impl CollectionView {
    /// Reads a `collection` fence.
    #[must_use]
    pub fn parse(fence: &Fence) -> Read<Self> {
        let mut problems = Problems::default();
        let mut view = Self::default();
        key_values(fence, &mut problems, |key, value| {
            match key {
                "id" => view.id = value,
                "source" => {
                    value
                        .strip_prefix("collection:")
                        .unwrap_or(&value)
                        .clone_into(&mut view.id);
                }
                "view" => view.view = Some(value),
                "filter" => view.filter = Some(value),
                _ => return false,
            }
            true
        });
        if view.id.is_empty() {
            problems.add(0, "a collection view needs `id:`");
        }
        problems.done(view)
    }
}
