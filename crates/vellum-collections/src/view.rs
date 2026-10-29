// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Views: table, board, gallery, calendar, timeline and list, with
//! filters, sorts, group-by and hidden columns.
//!
//! Filters and sorts are written as short text so they read well in
//! `schema.yaml` and in `collection` fences:
//!
//! | Filter | Meaning |
//! |---|---|
//! | `Status is Done`, `Status == Done` | equals (any item, for lists) |
//! | `Status not Done`, `Status is not Done`, `Status != Done` | does not equal |
//! | `Title contains replay`, `Title not contains x` | substring, ignoring case |
//! | `Owner is empty`, `Owner is not empty` | blank or not |
//! | `Estimate > 3`, `Target <= @today` | ordered comparison |
//!
//! Sorts are `Target` or `Target desc`. A property can be named by key or
//! display name; quote names that contain spaces and operator words.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use vellum_kernel::{CollectionId, DocId};

use crate::compute::{Cell, CellError, Computer};
use crate::error::SchemaError;
use crate::formula::percent;
use crate::row::{parse_bool, parse_number};
use crate::schema::{NumberFormat, Property, PropertyType, Schema, SelectOption};
use crate::telemetry;
use crate::value::{DateValue, Value};

/// Most rows one view query returns.
pub const MAX_LIMIT: usize = 10_000;

/// How a view lays out rows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    /// Rows and columns.
    #[default]
    Table,
    /// Columns of cards, one per group.
    Board,
    /// A grid of cards.
    Gallery,
    /// Rows placed on a month grid by a date.
    Calendar,
    /// Rows as bars between two dates.
    Timeline,
    /// A compact list of titles.
    List,
}

impl ViewKind {
    /// The kind's name in `schema.yaml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Board => "board",
            Self::Gallery => "gallery",
            Self::Calendar => "calendar",
            Self::Timeline => "timeline",
            Self::List => "list",
        }
    }
}

/// A filter operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Equals (any item, for lists).
    Is,
    /// Does not equal.
    Not,
    /// Contains, ignoring case.
    Contains,
    /// Does not contain.
    NotContains,
    /// Is blank.
    Empty,
    /// Is not blank.
    NotEmpty,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
}

impl FilterOp {
    const fn text(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::Not => "is not",
            Self::Contains => "contains",
            Self::NotContains => "not contains",
            Self::Empty => "is empty",
            Self::NotEmpty => "is not empty",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

/// One filter; a view keeps rows matching all of its filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Filter {
    /// The property key.
    pub key: String,
    /// The operator.
    pub op: FilterOp,
    /// The operand as written (blank for `is empty`).
    pub value: String,
}

const OPS: [(&str, FilterOp, bool); 15] = [
    (" is not empty", FilterOp::NotEmpty, true),
    (" is empty", FilterOp::Empty, true),
    (" not contains ", FilterOp::NotContains, false),
    (" contains ", FilterOp::Contains, false),
    (" is not ", FilterOp::Not, false),
    (" is ", FilterOp::Is, false),
    (" not ", FilterOp::Not, false),
    ("!=", FilterOp::Not, false),
    ("≠", FilterOp::Not, false),
    ("==", FilterOp::Is, false),
    ("<=", FilterOp::Le, false),
    (">=", FilterOp::Ge, false),
    ("<", FilterOp::Lt, false),
    (">", FilterOp::Gt, false),
    ("=", FilterOp::Is, false),
];

impl Filter {
    /// Parses filter text such as `Status != Done` against a schema.
    ///
    /// # Errors
    ///
    /// Returns a message when there is no operator or no such property.
    pub fn parse(text: &str, schema: &Schema) -> Result<Self, String> {
        let text = text.trim();
        // A quoted property name may contain anything.
        let (name, rest, offset) = if let Some(q) = text.strip_prefix('"') {
            let end = q
                .find('"')
                .ok_or("the property name is missing its closing quote")?;
            (
                q.get(..end).unwrap_or_default(),
                q.get(end + 1..).unwrap_or_default(),
                None,
            )
        } else {
            ("", text, Some(()))
        };
        let haystack = format!("{rest} ");
        let lower = haystack.to_ascii_lowercase();
        let mut best: Option<(usize, &str, FilterOp, bool)> = None;
        for (needle, op, unary) in OPS {
            if let Some(at) = lower.find(needle) {
                let better =
                    best.is_none_or(|(b, n, ..)| at < b || (at == b && needle.len() > n.len()));
                if better {
                    best = Some((at, needle, op, unary));
                }
            }
        }
        let (at, needle, op, unary) =
            best.ok_or_else(|| format!("no operator in filter {text:?}"))?;
        let prop_text = if offset.is_some() {
            haystack.get(..at).unwrap_or_default()
        } else {
            name
        };
        let value = haystack.get(at + needle.len()..).unwrap_or_default().trim();
        if unary && !value.is_empty() {
            return Err(format!("{} takes no value", op.text()));
        }
        let p = schema
            .resolve(prop_text.trim())
            .ok_or_else(|| format!("no property {:?}", prop_text.trim()))?;
        Ok(Self {
            key: p.key.clone(),
            op,
            value: unquote(value).to_owned(),
        })
    }

    pub(crate) fn to_text(&self) -> String {
        let value = if self.value.trim() != self.value || self.value.contains(' ') {
            format!("\"{}\"", self.value)
        } else {
            self.value.clone()
        };
        match self.op {
            FilterOp::Empty | FilterOp::NotEmpty => format!("{} {}", self.key, self.op.text()),
            op => format!("{} {} {value}", self.key, op.text()),
        }
    }
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// One sort key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sort {
    /// The property key.
    pub key: String,
    /// Largest or latest first.
    pub descending: bool,
}

impl Sort {
    /// Parses `Target` or `Target desc` (also `asc`, or a leading `-`).
    ///
    /// # Errors
    ///
    /// Returns a message when the property does not exist.
    pub fn parse(text: &str, schema: &Schema) -> Result<Self, String> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        let (name, descending) = text.strip_prefix('-').map_or_else(
            || {
                if lower.ends_with(" desc") {
                    (text.get(..text.len() - 5).unwrap_or_default(), true)
                } else if lower.ends_with(" asc") {
                    (text.get(..text.len() - 4).unwrap_or_default(), false)
                } else {
                    (text, false)
                }
            },
            |n| (n, true),
        );
        let p = schema
            .resolve(unquote(name.trim()))
            .ok_or_else(|| format!("no property {:?}", name.trim()))?;
        Ok(Self {
            key: p.key.clone(),
            descending,
        })
    }

    pub(crate) fn to_text(&self) -> String {
        if self.descending {
            format!("{} desc", self.key)
        } else {
            self.key.clone()
        }
    }
}

/// A saved or ad-hoc view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewDef {
    /// The view's name, unique in the collection.
    pub name: String,
    /// The layout.
    pub kind: ViewKind,
    /// Rows must match every filter.
    pub filters: Vec<Filter>,
    /// Sort keys, most significant first; rows tie-break by creation.
    pub sorts: Vec<Sort>,
    /// Group-by property key (board columns; table and list sections).
    pub group: Option<String>,
    /// Hidden property keys; the title is never hidden.
    pub hidden: Vec<String>,
    /// Date property for calendar and timeline (default: the first date).
    pub date: Option<String>,
    /// End-date property for timeline bars.
    pub end: Option<String>,
    /// Most rows to return.
    pub limit: Option<usize>,
}

impl ViewDef {
    /// The table view every collection starts with.
    #[must_use]
    pub fn default_table() -> Self {
        Self {
            name: "All".into(),
            ..Self::default()
        }
    }

    /// Builds a view from its `schema.yaml` (or fence) form.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::View`] naming the bad filter, sort or key.
    pub(crate) fn from_raw(raw: &ViewDefRaw, schema: &Schema) -> Result<Self, SchemaError> {
        let name = raw.name.clone().unwrap_or_else(|| "All".into());
        let err = |message: String| SchemaError::View {
            view: name.clone(),
            message,
        };
        let key = |text: &str| {
            schema
                .resolve(text)
                .map(|p| p.key.clone())
                .ok_or_else(|| err(format!("no property {text:?}")))
        };
        Ok(Self {
            kind: raw.kind.unwrap_or_default(),
            filters: raw
                .filter
                .items()
                .iter()
                .map(|f| Filter::parse(f, schema).map_err(&err))
                .collect::<Result<_, _>>()?,
            sorts: raw
                .sort
                .items()
                .iter()
                .map(|s| Sort::parse(s, schema).map_err(&err))
                .collect::<Result<_, _>>()?,
            group: raw.group.as_deref().map(key).transpose()?,
            hidden: raw
                .hide
                .items()
                .iter()
                .map(|h| key(h))
                .collect::<Result<_, _>>()?,
            date: raw.date.as_deref().map(key).transpose()?,
            end: raw.end.as_deref().map(key).transpose()?,
            limit: raw.limit.map(|l| l.min(MAX_LIMIT)),
            name,
        })
    }

    pub(crate) fn to_raw(&self, _schema: &Schema) -> ViewDefRaw {
        let list = |items: Vec<String>| match items.len() {
            0 => OneOrMany::None,
            1 => OneOrMany::One(items.into_iter().next().unwrap_or_default()),
            _ => OneOrMany::Many(items),
        };
        ViewDefRaw {
            name: Some(self.name.clone()),
            kind: Some(self.kind),
            filter: list(self.filters.iter().map(Filter::to_text).collect()),
            sort: list(self.sorts.iter().map(Sort::to_text).collect()),
            group: self.group.clone(),
            hide: list(self.hidden.clone()),
            date: self.date.clone(),
            end: self.end.clone(),
            limit: self.limit,
        }
    }

    /// Applies fence-style overrides on top of this view.
    pub(crate) fn overlay(&mut self, raw: &ViewDefRaw, schema: &Schema) -> Result<(), SchemaError> {
        let extra = Self::from_raw(raw, schema)?;
        self.filters.extend(extra.filters);
        if !extra.sorts.is_empty() {
            self.sorts = extra.sorts;
        }
        if raw.kind.is_some() {
            self.kind = extra.kind;
        }
        if extra.group.is_some() {
            self.group = extra.group;
        }
        if !extra.hidden.is_empty() {
            self.hidden = extra.hidden;
        }
        if extra.date.is_some() {
            self.date = extra.date;
        }
        if extra.end.is_some() {
            self.end = extra.end;
        }
        if extra.limit.is_some() {
            self.limit = extra.limit;
        }
        Ok(())
    }
}

/// A YAML field that may be one string or a list of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub(crate) fn items(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(s) => vec![s.clone()],
            Self::Many(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ViewDefRaw {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<ViewKind>,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub(crate) filter: OneOrMany,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub(crate) sort: OneOrMany,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) group: Option<String>,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub(crate) hide: OneOrMany,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) limit: Option<usize>,
}

/// A column of a view result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    /// Property key.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Property type name (`number`, `select`…).
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Number display format, for numeric columns.
    pub format: NumberFormat,
    /// Options, for select and multi-select columns.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// Whether the value can be edited (stored, not computed).
    pub editable: bool,
}

/// One cell of a view result.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CellView {
    /// The typed value (absent on error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// The value as display text: numbers formatted, relations as titles.
    pub display: String,
    /// The inline error, if the cell failed to compute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CellError>,
}

/// Start and end dates of a calendar entry or timeline bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct DateSpan {
    /// Start.
    pub start: DateValue,
    /// End (equal to start for single-day entries).
    pub end: DateValue,
}

/// One row of a view result.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RowView {
    /// The row page.
    pub id: DocId,
    /// The title text.
    pub title: String,
    /// Cells, aligned with [`ViewResult::columns`].
    pub cells: Vec<CellView>,
    /// Dates, for calendar and timeline views.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<DateSpan>,
}

/// A group of rows (one board column or table section).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// The group's value; `None` for the "no value" group or an ungrouped view.
    pub value: Option<Value>,
    /// The heading to show.
    pub label: String,
    /// The rows.
    pub rows: Vec<RowView>,
}

/// The result of running a view.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewResult {
    /// The collection.
    pub collection: CollectionId,
    /// The collection title.
    pub title: String,
    /// The view name.
    pub view: String,
    /// The layout.
    pub kind: ViewKind,
    /// Visible columns, title first.
    pub columns: Vec<Column>,
    /// Groups; a single unlabelled group when the view is not grouped.
    pub groups: Vec<Group>,
    /// Rows matching the filters, before the limit.
    pub total: usize,
    /// Whether the limit cut rows off.
    pub truncated: bool,
}

/// Why a view could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryError {
    /// The collection is not loaded.
    #[error("collection {0} not found")]
    UnknownCollection(CollectionId),
    /// The named view does not exist.
    #[error("no view named {0:?}")]
    UnknownView(String),
    /// The view or its overrides are invalid.
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

impl From<QueryError> for vellum_kernel::Error {
    fn from(error: QueryError) -> Self {
        let kind = match &error {
            QueryError::UnknownCollection(_) | QueryError::UnknownView(_) => {
                vellum_kernel::ErrorKind::NotFound
            }
            QueryError::Schema(_) => vellum_kernel::ErrorKind::InvalidInput,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}

/// Runs a view over a collection, returning at most [`MAX_LIMIT`] rows.
///
/// # Errors
///
/// Returns [`QueryError::UnknownCollection`] when the collection is not
/// loaded.
pub fn query(
    computer: &mut Computer<'_>,
    collection: CollectionId,
    view: &ViewDef,
) -> Result<ViewResult, QueryError> {
    run(computer, collection, view, MAX_LIMIT)
}

/// Runs a view with a row cap (exports pass `usize::MAX`).
pub(crate) fn run(
    computer: &mut Computer<'_>,
    collection: CollectionId,
    view: &ViewDef,
    cap: usize,
) -> Result<ViewResult, QueryError> {
    let started = Instant::now();
    let span = tracing::info_span!(
        "collections.query",
        collection = %collection,
        view = %view.name,
        kind = view.kind.name(),
        rows = tracing::field::Empty,
    );
    let _guard = span.enter();
    let ws = computer.workspace();
    let c = ws
        .collection(collection)
        .ok_or(QueryError::UnknownCollection(collection))?;
    let schema = c.schema();

    let mut keyed = select(computer, collection, schema, view);
    let total = keyed.len();
    let limit = view.limit.unwrap_or(cap).min(cap);
    let truncated = total > limit;
    keyed.truncate(limit);

    // Columns.
    let columns: Vec<&Property> = schema
        .properties
        .iter()
        .filter(|p| matches!(p.kind, PropertyType::Title) || !view.hidden.contains(&p.key))
        .collect();
    let date_key = view.date.clone().or_else(|| {
        schema
            .properties
            .iter()
            .find(|p| matches!(p.kind, PropertyType::Date))
            .map(|p| p.key.clone())
    });

    // Rows.
    let mut views = Vec::with_capacity(keyed.len());
    for id in &keyed {
        let cells = columns
            .iter()
            .map(|p| {
                let cell = computer.value(collection, *id, &p.key);
                cell_view(computer, p, cell)
            })
            .collect();
        let span = if matches!(view.kind, ViewKind::Calendar | ViewKind::Timeline) {
            date_key.as_deref().and_then(|k| {
                let start = computer.value(collection, *id, k).ok()?.as_date()?;
                let end = view
                    .end
                    .as_deref()
                    .and_then(|e| computer.value(collection, *id, e).ok()?.as_date())
                    .filter(|e| *e >= start)
                    .unwrap_or(start);
                Some(DateSpan { start, end })
            })
        } else {
            None
        };
        let title = c.row(*id).map(|r| r.title(schema)).unwrap_or_default();
        views.push(RowView {
            id: *id,
            title,
            cells,
            span,
        });
    }

    let groups = match view.group.as_deref().and_then(|g| schema.property(g)) {
        Some(p) => group(computer, collection, p, views, view.kind == ViewKind::Board),
        None => vec![Group {
            value: None,
            label: String::new(),
            rows: views,
        }],
    };

    let _ = span.record("rows", total);
    telemetry::queried(view.kind.name(), started);
    Ok(ViewResult {
        collection,
        title: schema.title.clone(),
        view: view.name.clone(),
        kind: view.kind,
        columns: columns
            .iter()
            .map(|p| Column {
                key: p.key.clone(),
                name: p.name.clone(),
                kind: p.kind.name(),
                format: number_format(p),
                options: p.kind.options().to_vec(),
                editable: p.kind.is_stored(),
            })
            .collect(),
        groups,
        total,
        truncated,
    })
}

/// Filters and sorts a collection's rows for a view.
fn select(
    computer: &mut Computer<'_>,
    collection: CollectionId,
    schema: &Schema,
    view: &ViewDef,
) -> Vec<DocId> {
    let ws = computer.workspace();
    let Some(c) = ws.collection(collection) else {
        return Vec::new();
    };
    // Filter.
    let mut rows: Vec<DocId> = Vec::new();
    for row in c.rows() {
        let mut keep = true;
        for f in &view.filters {
            let Some(p) = schema.property(&f.key) else {
                continue;
            };
            let cell = computer.value(collection, row.id, &f.key);
            if !matches(computer, p, &cell, f) {
                keep = false;
                break;
            }
        }
        if keep {
            rows.push(row.id);
        }
    }

    // Sort: each key in turn, empty values last either way, then creation.
    let mut keyed: Vec<(DocId, Vec<Value>)> = rows
        .iter()
        .map(|id| {
            let vals = view
                .sorts
                .iter()
                .map(|s| computer.value(collection, *id, &s.key).unwrap_or_default())
                .collect();
            (*id, vals)
        })
        .collect();
    keyed.sort_by(|(a_id, a), (b_id, b)| {
        for ((s, x), y) in view.sorts.iter().zip(a).zip(b) {
            let p = schema.property(&s.key);
            let ord = match (x.is_empty(), y.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    let o = compare_for_sort(p, x, y);
                    if s.descending { o.reverse() } else { o }
                }
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a_id.cmp(b_id)
    });
    keyed.into_iter().map(|(id, _)| id).collect()
}

const fn number_format(p: &Property) -> NumberFormat {
    match &p.kind {
        PropertyType::Number { format }
        | PropertyType::Rollup { format, .. }
        | PropertyType::Formula { format, .. } => *format,
        _ => NumberFormat::Plain,
    }
}

fn option_index(p: Option<&Property>, v: &Value) -> Option<usize> {
    let name = v.as_text()?;
    p?.kind.options().iter().position(|o| o.name == name)
}

pub(crate) fn compare_for_sort(prop: Option<&Property>, a: &Value, b: &Value) -> Ordering {
    match (option_index(prop, a), option_index(prop, b)) {
        (Some(i), Some(j)) => i.cmp(&j),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.sort_cmp(b),
    }
}

/// Display text for each item of a cell (relation items become titles).
pub(crate) fn item_texts(computer: &Computer<'_>, p: &Property, v: &Value) -> Vec<String> {
    match &p.kind {
        PropertyType::Relation { to, .. } => {
            let target = computer.workspace().collection(*to);
            v.items()
                .iter()
                .filter_map(|i| {
                    let id: DocId = i.as_text()?.parse().ok()?;
                    let title = target
                        .and_then(|c| c.row(id).map(|r| r.title(c.schema())))
                        .unwrap_or_else(|| id.to_string());
                    Some(title)
                })
                .collect()
        }
        _ => v
            .leaves()
            .into_iter()
            .map(|i| display_value(p, i))
            .collect(),
    }
}

fn display_value(p: &Property, v: &Value) -> String {
    match (v, number_format(p)) {
        (Value::Number(n), NumberFormat::Percent) => percent(*n, 0),
        (Value::Bool(b), _) => if *b { "Yes" } else { "No" }.to_owned(),
        _ => v.to_string(),
    }
}

/// The display text of a cell.
pub(crate) fn display(computer: &Computer<'_>, p: &Property, cell: &Cell) -> String {
    match cell {
        Ok(Value::Bool(b)) => display_value(p, &Value::Bool(*b)),
        Ok(v) => item_texts(computer, p, v).join(", "),
        Err(e) => format!("⚠ {}", e.message),
    }
}

fn cell_view(computer: &Computer<'_>, p: &Property, cell: Cell) -> CellView {
    let display = display(computer, p, &cell);
    match cell {
        Ok(v) => CellView {
            value: (!v.is_empty()).then_some(v),
            display,
            error: None,
        },
        Err(e) => CellView {
            value: None,
            display,
            error: Some(e),
        },
    }
}

fn matches(computer: &Computer<'_>, p: &Property, cell: &Cell, f: &Filter) -> bool {
    let v = cell.as_ref().cloned().unwrap_or_default();
    let texts = item_texts(computer, p, &v);
    let needle = f.value.to_lowercase();
    match f.op {
        FilterOp::Empty => v.is_empty(),
        FilterOp::NotEmpty => !v.is_empty(),
        FilterOp::Is => equals(computer, p, &v, &texts, &f.value),
        FilterOp::Not => !equals(computer, p, &v, &texts, &f.value),
        FilterOp::Contains => texts.iter().any(|t| t.to_lowercase().contains(&needle)),
        FilterOp::NotContains => !texts.iter().any(|t| t.to_lowercase().contains(&needle)),
        FilterOp::Lt | FilterOp::Le | FilterOp::Gt | FilterOp::Ge => {
            let Some(ord) = order(computer, &v, &f.value) else {
                return false;
            };
            match f.op {
                FilterOp::Lt => ord == Ordering::Less,
                FilterOp::Le => ord != Ordering::Greater,
                FilterOp::Gt => ord == Ordering::Greater,
                _ => ord != Ordering::Less,
            }
        }
    }
}

fn operand_date(computer: &Computer<'_>, text: &str) -> Option<DateValue> {
    match text.trim().to_lowercase().as_str() {
        "@today" | "today" => Some(DateValue::day(computer.now().date())),
        "@now" | "now" => Some(computer.now()),
        other => DateValue::parse(other),
    }
}

fn equals(
    computer: &Computer<'_>,
    p: &Property,
    v: &Value,
    texts: &[String],
    operand: &str,
) -> bool {
    if operand.trim().is_empty() {
        return v.is_empty();
    }
    match v {
        Value::Bool(b) => parse_bool(operand) == Some(*b),
        Value::Empty if matches!(p.kind, PropertyType::Checkbox) => {
            parse_bool(operand) == Some(false)
        }
        Value::Number(n) => parse_number(operand).is_some_and(|x| (x - n).abs() < 1e-9),
        Value::Date(d) => operand_date(computer, operand).is_some_and(|x| x.date() == d.date()),
        _ => {
            let want = operand.to_lowercase();
            texts.iter().any(|t| t.to_lowercase() == want)
        }
    }
}

fn order(computer: &Computer<'_>, v: &Value, operand: &str) -> Option<Ordering> {
    match v {
        Value::Number(n) => parse_number(operand).map(|x| n.total_cmp(&x)),
        Value::Date(d) => operand_date(computer, operand).map(|x| {
            if x.time().is_none() {
                d.date().cmp(&x.date())
            } else {
                d.cmp(&x)
            }
        }),
        Value::Text(t) => Some(t.to_lowercase().cmp(&operand.to_lowercase())),
        _ => None,
    }
}

fn group(
    computer: &mut Computer<'_>,
    collection: CollectionId,
    p: &Property,
    rows: Vec<RowView>,
    board: bool,
) -> Vec<Group> {
    // Collect each row's group values.
    let mut keyed: Vec<(Vec<(Value, String)>, RowView)> = Vec::with_capacity(rows.len());
    for r in rows {
        let cell = computer.value(collection, r.id, &p.key).unwrap_or_default();
        let items: Vec<(Value, String)> = match (&p.kind, &cell) {
            (PropertyType::Checkbox, v) => {
                let b = v.truthy();
                vec![(Value::Bool(b), display_value(p, &Value::Bool(b)))]
            }
            (_, v) if v.is_empty() => Vec::new(),
            (_, v) => {
                let texts = item_texts(computer, p, v);
                let leaves: Vec<Value> = match &p.kind {
                    PropertyType::Relation { .. } => v.items().to_vec(),
                    _ => v.leaves().into_iter().cloned().collect(),
                };
                let mut seen = BTreeSet::new();
                leaves
                    .into_iter()
                    .zip(texts)
                    .filter(|(_, t)| seen.insert(t.clone()))
                    .collect()
            }
        };
        keyed.push((items, r));
    }
    // Order groups: options first (all of them on a board), then other
    // values in sort order, then the empty group.
    let mut order: Vec<(Value, String)> = Vec::new();
    if board {
        for o in p.kind.options() {
            order.push((Value::Text(o.name.clone()), o.name.clone()));
        }
    }
    let mut extra: Vec<(Value, String)> = Vec::new();
    for (items, _) in &keyed {
        for (v, label) in items {
            if !order.iter().chain(&extra).any(|(_, l)| l == label) {
                extra.push((v.clone(), label.clone()));
            }
        }
    }
    extra.sort_by(|(a, _), (b, _)| compare_for_sort(Some(p), a, b));
    order.extend(extra);
    let mut groups: Vec<Group> = order
        .into_iter()
        .map(|(value, label)| Group {
            value: Some(value),
            label,
            rows: Vec::new(),
        })
        .collect();
    let mut none = Group {
        value: None,
        label: format!("No {}", p.name),
        rows: Vec::new(),
    };
    for (items, row) in keyed {
        if items.is_empty() {
            none.rows.push(row);
            continue;
        }
        for (_, label) in &items {
            if let Some(g) = groups.iter_mut().find(|g| &g.label == label) {
                g.rows.push(row.clone());
            }
        }
    }
    if !none.rows.is_empty() || (board && groups.is_empty()) {
        groups.push(none);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::TEST_SCHEMA;

    #[test]
    fn filters_parse() {
        let s = Schema::from_yaml(TEST_SCHEMA).unwrap();
        let f = Filter::parse("Priority != P2", &s).unwrap();
        assert_eq!(
            (f.key.as_str(), f.op, f.value.as_str()),
            ("pr", FilterOp::Not, "P2")
        );
        let f = Filter::parse("Requirement contains \"replay days\"", &s).unwrap();
        assert_eq!(
            (f.op, f.value.as_str()),
            (FilterOp::Contains, "replay days")
        );
        let f = Filter::parse("Requirement is not empty", &s).unwrap();
        assert_eq!(f.op, FilterOp::NotEmpty);
        let f = Filter::parse("Estimate >= 3", &s).unwrap();
        assert_eq!((f.key.as_str(), f.op), ("est", FilterOp::Ge));
        let f = Filter::parse("\"Priority\" is P0", &s).unwrap();
        assert_eq!(f.op, FilterOp::Is);
        assert!(Filter::parse("Nope is 1", &s).is_err());
        assert!(Filter::parse("Priority", &s).is_err());
        assert!(Filter::parse("Priority is empty now", &s).is_err());
        let again = Filter::parse(&f.to_text(), &s).unwrap();
        assert_eq!(again, f);
    }

    #[test]
    fn sorts_parse() {
        let s = Schema::from_yaml(TEST_SCHEMA).unwrap();
        assert!(Sort::parse("Estimate desc", &s).unwrap().descending);
        assert!(Sort::parse("-est", &s).unwrap().descending);
        assert!(!Sort::parse("Priority", &s).unwrap().descending);
    }
}
