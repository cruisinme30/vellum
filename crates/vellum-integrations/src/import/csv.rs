// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A small RFC 4180 CSV reader with header lookup.

use std::collections::BTreeMap;

use super::ImportError;

/// Splits CSV text into rows of fields. Quoted fields may contain commas,
/// quotes (doubled) and line breaks; a UTF-8 byte-order mark is skipped.
///
/// # Errors
///
/// [`ImportError::Unreadable`] for an unterminated quoted field.
pub fn rows(text: &str) -> Result<Vec<Vec<String>>, ImportError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    let _ = chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(ImportError::Unreadable(
            "unterminated quoted field".to_owned(),
        ));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r.first().is_some_and(String::is_empty)));
    Ok(rows)
}

/// A CSV file with named columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    columns: BTreeMap<String, usize>,
    /// The data rows.
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Parses CSV whose first row is the header.
    ///
    /// # Errors
    ///
    /// [`ImportError::Unreadable`] for malformed CSV or an empty file.
    pub fn parse(text: &str) -> Result<Self, ImportError> {
        let mut all = rows(text)?.into_iter();
        let header = all
            .next()
            .ok_or_else(|| ImportError::Unreadable("empty file".to_owned()))?;
        let columns = header
            .iter()
            .enumerate()
            .map(|(i, name)| (name.trim().to_ascii_lowercase(), i))
            .collect();
        Ok(Self {
            columns,
            rows: all.collect(),
        })
    }

    /// Whether a column exists (case-insensitive).
    #[must_use]
    pub fn has(&self, column: &str) -> bool {
        self.columns.contains_key(&column.to_ascii_lowercase())
    }

    /// A row's value in the first of `columns` that exists, trimmed.
    #[must_use]
    pub fn get<'r>(&self, row: &'r [String], columns: &[&str]) -> &'r str {
        columns
            .iter()
            .find_map(|c| self.columns.get(&c.to_ascii_lowercase()))
            .and_then(|i| row.get(*i))
            .map_or("", |v| v.trim())
    }

    /// Like [`Self::get`], `None` when empty.
    #[must_use]
    pub fn opt(&self, row: &[String], columns: &[&str]) -> Option<String> {
        let value = self.get(row, columns);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_line_breaks() -> Result<(), ImportError> {
        let text =
            "\u{feff}a,b,c\r\n1,\"two, \"\"2\"\"\",\"multi\nline\"\r\n\r\n4,,6";
        let parsed = rows(text)?;
        assert_eq!(
            parsed,
            vec![
                vec!["a", "b", "c"],
                vec!["1", "two, \"2\"", "multi\nline"],
                vec!["4", "", "6"],
            ]
        );
        assert!(rows("\"open").is_err());
        Ok(())
    }

    #[test]
    fn header_lookup() -> Result<(), ImportError> {
        let table = Table::parse("ID,Title , Due Date\nENG-1, Fix ,\n")?;
        let row = table.rows.first().cloned().unwrap_or_default();
        assert!(table.has("title"));
        assert_eq!(table.get(&row, &["Name", "Title"]), "Fix");
        assert_eq!(table.opt(&row, &["Due Date"]), None);
        assert!(Table::parse("").is_err());
        Ok(())
    }
}
