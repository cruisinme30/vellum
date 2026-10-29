// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! CSV import and export (RFC 4180, UTF-8).
//!
//! Export writes a view's visible columns as display text: numbers as
//! shown, checkboxes as `Yes`/`No`, relations as the related rows' titles,
//! computed columns as their current values. Text that a spreadsheet would
//! run as a formula (starting `=`, `+`, `-`, `@`, tab or CR) is prefixed
//! with `'`.
//!
//! Import either creates a new collection, inferring each column's type
//! from its values, or adds rows to an existing one, matching headers to
//! property names or keys. Bad cells never abort an import: they are left
//! empty and reported as diagnostics with their line number.

use std::collections::{BTreeMap, BTreeSet};

use vellum_kernel::{CollectionId, DocId};

use crate::compute::Computer;
use crate::error::Diagnostic;
use crate::row::{Row, parse_bool, parse_number, relation_value, slug};
use crate::schema::{
    MAX_KEY_LEN, MAX_PROPERTIES, NumberFormat, Property, PropertyType, RESERVED_KEYS, Schema,
    SelectOption,
};
use crate::telemetry;
use crate::value::{DateValue, Value};
use crate::view::{self, QueryError, ViewDef};
use crate::workspace::{Collection, Edit, Workspace};

/// Most data rows one import accepts.
pub const MAX_IMPORT_ROWS: usize = 100_000;

/// Most distinct values for a column to be inferred as a select.
const SELECT_MAX_OPTIONS: usize = 20;

/// Why a CSV file could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CsvError {
    /// The file is not valid CSV.
    #[error("CSV is not valid: {0}")]
    Malformed(String),
    /// The file has no header row.
    #[error("CSV has no header row")]
    NoHeader,
    /// More rows than [`MAX_IMPORT_ROWS`].
    #[error("CSV has more than {MAX_IMPORT_ROWS} rows")]
    TooManyRows,
    /// More columns than a collection may have.
    #[error("CSV has more than {MAX_PROPERTIES} columns")]
    TooManyColumns,
    /// The target collection is not loaded.
    #[error("collection {0} not found")]
    UnknownCollection(CollectionId),
    /// The export view could not run.
    #[error(transparent)]
    Query(#[from] QueryError),
}

impl From<CsvError> for vellum_kernel::Error {
    fn from(error: CsvError) -> Self {
        let kind = match &error {
            CsvError::UnknownCollection(_) => vellum_kernel::ErrorKind::NotFound,
            _ => vellum_kernel::ErrorKind::InvalidInput,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}

/// Exports a view as CSV.
///
/// # Errors
///
/// Returns [`CsvError::Query`] when the collection is not loaded.
pub fn export(
    computer: &mut Computer<'_>,
    collection: CollectionId,
    view: &ViewDef,
) -> Result<String, CsvError> {
    let span = tracing::info_span!("collections.csv_export", collection = %collection);
    let _guard = span.enter();
    let result = view::run(computer, collection, view, usize::MAX)?;
    let mut w = ::csv::Writer::from_writer(Vec::new());
    let header: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
    w.write_record(&header)
        .map_err(|e| CsvError::Malformed(e.to_string()))?;
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in result.groups.iter().flat_map(|g| &g.rows) {
        if !seen.insert(row.id) {
            continue;
        }
        let record: Vec<String> = row
            .cells
            .iter()
            .map(|c| match (&c.value, &c.error) {
                (_, Some(_)) | (None, None) => String::new(),
                (Some(Value::Number(_)), None) => c.display.clone(),
                (Some(_), None) => defuse(&c.display),
            })
            .collect();
        w.write_record(&record)
            .map_err(|e| CsvError::Malformed(e.to_string()))?;
        count += 1;
    }
    let bytes = w
        .into_inner()
        .map_err(|e| CsvError::Malformed(e.to_string()))?;
    telemetry::csv("export", count);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Stops spreadsheets from running cell text as a formula.
fn defuse(text: &str) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

/// The result of reading a CSV file.
#[derive(Debug, Clone)]
pub struct CsvImport {
    /// The schema: inferred for a new collection, or the existing one.
    pub schema: Schema,
    /// One value map per data row, keyed by property key.
    pub rows: Vec<BTreeMap<String, Value>>,
    /// Headers that matched no writable property and were ignored.
    pub skipped_columns: Vec<String>,
    /// Problems with individual cells.
    pub diagnostics: Vec<Diagnostic>,
}

impl CsvImport {
    /// The edits that add the imported rows to an existing collection.
    #[must_use]
    pub fn edits(&self) -> Vec<Edit> {
        self.rows
            .iter()
            .map(|values| Edit::CreateRow {
                collection: self.schema.id,
                id: None,
                values: values.clone(),
                body: String::new(),
            })
            .collect()
    }

    /// Builds a new collection from an import made by [`import_new`].
    #[must_use]
    pub fn into_collection(self) -> Collection {
        let mut c = Collection::new(self.schema);
        for values in self.rows {
            let mut row = Row::new(DocId::generate());
            for (k, v) in values {
                row.set_raw(&k, v);
            }
            c.insert_row(row);
        }
        c
    }
}

fn read(text: &str) -> Result<(Vec<String>, Vec<Vec<String>>), CsvError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut r = ::csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());
    let mut records = r.records();
    let header: Vec<String> = match records.next() {
        Some(Ok(h)) => h.iter().map(|s| s.trim().to_owned()).collect(),
        Some(Err(e)) => return Err(CsvError::Malformed(e.to_string())),
        None => return Err(CsvError::NoHeader),
    };
    if header.len() > MAX_PROPERTIES {
        return Err(CsvError::TooManyColumns);
    }
    let mut rows = Vec::new();
    for rec in records {
        let rec = rec.map_err(|e| CsvError::Malformed(e.to_string()))?;
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        if rows.len() == MAX_IMPORT_ROWS {
            return Err(CsvError::TooManyRows);
        }
        let mut cells: Vec<String> = rec.iter().map(|c| undefuse(c.trim()).to_owned()).collect();
        cells.resize(header.len(), String::new());
        rows.push(cells);
    }
    Ok((header, rows))
}

fn undefuse(text: &str) -> &str {
    match text.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@']) => rest,
        _ => text,
    }
}

fn csv_number(text: &str) -> Option<(f64, bool)> {
    text.strip_suffix('%').map_or_else(
        || parse_number(&text.replace(',', "")).map(|n| (n, false)),
        |p| parse_number(p).map(|n| (n / 100.0, true)),
    )
}

fn strict_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "✓" => Some(true),
        "false" | "no" => Some(false),
        _ => None,
    }
}

/// Reads a CSV file as a new collection, inferring property types. The
/// first column becomes the title.
///
/// # Errors
///
/// Returns a [`CsvError`] when the file cannot be read.
pub fn import_new(text: &str, id: CollectionId, title: &str) -> Result<CsvImport, CsvError> {
    let span = tracing::info_span!("collections.csv_import", collection = %id, new = true);
    let _guard = span.enter();
    let (header, rows) = read(text)?;
    let mut properties: Vec<Property> = Vec::with_capacity(header.len());
    let mut keys = BTreeSet::new();
    let mut names = BTreeSet::new();
    for (i, h) in header.iter().enumerate() {
        let column: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.get(i).map(String::as_str))
            .filter(|c| !c.is_empty())
            .collect();
        let kind = if i == 0 {
            PropertyType::Title
        } else {
            infer(&column, rows.len())
        };
        properties.push(Property {
            key: unique_key(h, i, &mut keys),
            name: unique_name(h, i, &mut names),
            kind,
        });
    }
    if properties.is_empty() {
        return Err(CsvError::NoHeader);
    }
    let mut schema = Schema::new(id, title);
    schema.properties = properties;
    let mut out = CsvImport {
        schema,
        rows: Vec::with_capacity(rows.len()),
        skipped_columns: Vec::new(),
        diagnostics: Vec::new(),
    };
    for r in &rows {
        let mut values = BTreeMap::new();
        for (p, cell) in out.schema.properties.iter().zip(r) {
            if let Some(v) = cell_value(&p.kind, cell) {
                let _ = values.insert(p.key.clone(), v);
            }
        }
        out.rows.push(values);
    }
    telemetry::csv("import", out.rows.len());
    Ok(out)
}

fn infer(column: &[&str], rows: usize) -> PropertyType {
    if column.is_empty() {
        return PropertyType::Text;
    }
    let numbers: Option<Vec<(f64, bool)>> = column.iter().map(|c| csv_number(c)).collect();
    if let Some(ns) = numbers {
        let format = if ns.iter().all(|(_, pct)| *pct) {
            NumberFormat::Percent
        } else {
            NumberFormat::Plain
        };
        return PropertyType::Number { format };
    }
    if column.iter().all(|c| strict_bool(c).is_some()) {
        return PropertyType::Checkbox;
    }
    if column.iter().all(|c| DateValue::parse(c).is_some()) {
        return PropertyType::Date;
    }
    if column
        .iter()
        .all(|c| c.starts_with("https://") || c.starts_with("http://"))
    {
        return PropertyType::Url;
    }
    let mut options: Vec<String> = Vec::new();
    for c in column {
        if !options.iter().any(|o| o == c) {
            options.push((*c).to_owned());
        }
    }
    let limit = SELECT_MAX_OPTIONS.min(rows / 2).max(1);
    if options.len() <= limit && column.iter().all(|c| c.chars().count() <= 40) {
        return PropertyType::Select {
            options: options
                .into_iter()
                .map(|name| SelectOption { name, color: None })
                .collect(),
        };
    }
    PropertyType::Text
}

fn unique_key(header: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let mut base: String = slug(header)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base = format!("c{base}");
    }
    if base.trim_matches('_') == "c" || base.chars().all(|c| c == '_') {
        base = format!("col_{}", index + 1);
    }
    base.truncate(MAX_KEY_LEN - 4);
    let mut key = base.clone();
    let mut n = 1;
    while RESERVED_KEYS.contains(&key.as_str()) || used.contains(&key) {
        n += 1;
        key = format!("{base}_{n}");
    }
    let _ = used.insert(key.clone());
    key
}

fn unique_name(header: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let base = if header.trim().is_empty() {
        format!("Column {}", index + 1)
    } else {
        header.trim().to_owned()
    };
    let mut name = base.clone();
    let mut n = 1;
    while used.contains(&name.to_lowercase()) {
        n += 1;
        name = format!("{base} {n}");
    }
    let _ = used.insert(name.to_lowercase());
    name
}

/// A typed value from CSV text, for types that need no lookup.
fn cell_value(kind: &PropertyType, cell: &str) -> Option<Value> {
    if cell.is_empty() {
        return None;
    }
    let split = || {
        Value::List(
            cell.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(Value::text)
                .collect(),
        )
    };
    match kind {
        PropertyType::Number { .. } => csv_number(cell).map(|(n, _)| Value::Number(n)),
        PropertyType::Checkbox => parse_bool(cell).map(Value::Bool),
        PropertyType::Date => DateValue::parse(cell).map(Value::Date),
        PropertyType::Multi { .. } => Some(split()),
        PropertyType::Person => match split() {
            Value::List(mut items) if items.len() == 1 => items.pop(),
            other => Some(other),
        },
        PropertyType::Title
        | PropertyType::Text
        | PropertyType::Url
        | PropertyType::Select { .. } => Some(Value::text(cell)),
        PropertyType::Relation { .. }
        | PropertyType::Rollup { .. }
        | PropertyType::Formula { .. }
        | PropertyType::Created => None,
    }
}

/// Reads a CSV file as rows for an existing collection. Headers match
/// property names (case-insensitive) or keys; relation cells match related
/// rows by title or ID.
///
/// # Errors
///
/// Returns a [`CsvError`] when the file cannot be read or the collection
/// is not loaded.
pub fn import_into(
    text: &str,
    ws: &Workspace,
    collection: CollectionId,
) -> Result<CsvImport, CsvError> {
    let span = tracing::info_span!("collections.csv_import", collection = %collection, new = false);
    let _guard = span.enter();
    let c = ws
        .collection(collection)
        .ok_or(CsvError::UnknownCollection(collection))?;
    let schema = c.schema();
    let (header, rows) = read(text)?;
    let mut skipped = Vec::new();
    let columns: Vec<Option<&Property>> = header
        .iter()
        .map(|h| {
            let p = schema.resolve(h).filter(|p| p.kind.is_stored());
            if p.is_none() {
                skipped.push(h.clone());
            }
            p
        })
        .collect();
    let mut diagnostics = Vec::new();
    let mut out_rows = Vec::with_capacity(rows.len());
    for (n, r) in rows.iter().enumerate() {
        let line = format!("line {}", n + 2);
        let mut values = BTreeMap::new();
        for (p, cell) in columns.iter().zip(r) {
            let Some(p) = p else { continue };
            if cell.is_empty() {
                continue;
            }
            let value = match &p.kind {
                PropertyType::Relation { to, .. } => {
                    relation_cell(ws, *to, cell, &line, &mut diagnostics)
                }
                kind => cell_value(kind, cell),
            };
            match value {
                Some(v) => {
                    let _ = values.insert(p.key.clone(), v);
                }
                None if !matches!(p.kind, PropertyType::Relation { .. }) => {
                    diagnostics.push(Diagnostic::error(
                        &line,
                        format!("{}: {cell:?} is not a valid {}", p.name, p.kind.name()),
                    ));
                }
                None => {}
            }
        }
        out_rows.push(values);
    }
    telemetry::csv("import", out_rows.len());
    Ok(CsvImport {
        schema: schema.clone(),
        rows: out_rows,
        skipped_columns: skipped,
        diagnostics,
    })
}

fn relation_cell(
    ws: &Workspace,
    to: CollectionId,
    cell: &str,
    line: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Value> {
    let target = ws.collection(to)?;
    let mut ids = Vec::new();
    for part in cell.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let lower = part.to_lowercase();
        let found = part
            .parse::<DocId>()
            .ok()
            .filter(|id| target.row(*id).is_some())
            .or_else(|| {
                target
                    .rows()
                    .find(|r| r.title(target.schema()).to_lowercase() == lower)
                    .map(|r| r.id)
            });
        match found {
            Some(id) if !ids.contains(&id) => ids.push(id),
            Some(_) => {}
            None => diagnostics.push(Diagnostic::warning(
                line,
                format!("no row titled {part:?} in {}", target.schema().title),
            )),
        }
    }
    (!ids.is_empty()).then(|| relation_value(&ids))
}
