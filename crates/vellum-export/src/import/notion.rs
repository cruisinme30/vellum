// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Notion "Markdown & CSV" exports.
//!
//! Notion names every page and folder `Title <32 hex id>`; the ids are
//! dropped so `Gateway 1a2b….md` becomes `Gateway.md`, and links between
//! pages (written as URL-encoded relative paths) are matched on the
//! original names before that. A database is a CSV file (`DB <id>.csv`,
//! or `DB <id>_all.csv` with every row) beside a folder of row pages; it
//! becomes a collection whose rows keep their page bodies. Large exports
//! arrive as several zips inside one, which are read as one tree.

use std::collections::HashMap;

use super::{Builder, archive, markdown_title};
use crate::error::ImportError;

const FORMAT: &str = "notion";

pub(super) fn import(bytes: &[u8]) -> Result<Builder, ImportError> {
    let mut entries = archive::read(FORMAT, bytes, true)?;
    archive::strip_common_root(&mut entries);
    let mut builder = Builder::new(FORMAT);

    // Databases: prefer `_all.csv`, which has rows hidden by the view.
    let mut tables: HashMap<String, (String, String)> = HashMap::new();
    for entry in &entries {
        let Some(base) = entry.path.strip_suffix(".csv") else {
            continue;
        };
        let (base, all) = base
            .strip_suffix("_all")
            .map_or((base, false), |b| (b, true));
        if all || !tables.contains_key(base) {
            let _ = tables.insert(
                base.to_owned(),
                (
                    entry.path.clone(),
                    String::from_utf8_lossy(&entry.bytes).into_owned(),
                ),
            );
        }
    }
    let mut table_index: HashMap<String, usize> = HashMap::new();
    let mut names: Vec<&String> = tables.keys().collect();
    names.sort();
    for base in names {
        let Some((_, csv)) = tables.get(base) else {
            continue;
        };
        let title = strip_id(super::file_name(base));
        let index = builder.table(&strip_path(base), &title, csv.clone());
        let _ = table_index.insert(base.clone(), index);
    }

    let mut pages = 0;
    for entry in entries {
        let lower = entry.path.to_lowercase();
        if super::has_ext(&lower, &["csv"]) {
            continue;
        }
        if super::has_ext(&lower, &["md"]) {
            let text = String::from_utf8_lossy(&entry.bytes).into_owned();
            let stem = super::stem(super::file_name(&entry.path));
            let table =
                table_index.get(super::links::parent(&entry.path)).copied();
            let title = markdown_title(&text, &strip_id(stem));
            let text = if table.is_some() {
                strip_properties(&text)
            } else {
                text
            };
            let out = format!("{}.md", strip_path(super::stem(&entry.path)));
            let page = builder.page(&entry.path, &out, &title, text);
            if let Some(table) = table {
                builder.row_of(page, table);
            }
            pages += 1;
        } else {
            let out = strip_path(&entry.path);
            builder.asset(&entry.path, &out, entry.bytes);
        }
    }
    if pages == 0 && table_index.is_empty() {
        return Err(ImportError::format(
            FORMAT,
            "the archive has no Markdown or CSV files",
        ));
    }
    Ok(builder)
}

/// Drops Notion's ids from every segment of `path`.
fn strip_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            let (stem, ext) = match segment.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() && ext.len() <= 8 => {
                    (stem, Some(ext))
                }
                _ => (segment, None),
            };
            let stem = strip_id(stem);
            ext.map_or_else(|| stem.clone(), |ext| format!("{stem}.{ext}"))
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `Gateway 1a2b3c…` (a space and 32 hex digits) becomes `Gateway`.
fn strip_id(name: &str) -> String {
    match name.rsplit_once(' ') {
        Some((title, id))
            if id.len() == 32
                && id.chars().all(|c| c.is_ascii_hexdigit())
                && !title.is_empty() =>
        {
            title.to_owned()
        }
        _ => name.to_owned(),
    }
}

/// A database row page lists its properties as `Name: value` lines after
/// the `# Title` line; the collection holds both, so the body keeps only
/// what follows.
fn strip_properties(text: &str) -> String {
    let mut lines = text.lines().peekable();
    let mut out = String::with_capacity(text.len());
    // The `# Title` line and the blank line after it.
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            let _ = lines.next();
        } else if line.starts_with("# ") {
            let _ = lines.next();
            break;
        } else {
            break;
        }
    }
    while lines.peek().is_some_and(|line| line.trim().is_empty()) {
        let _ = lines.next();
    }
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            break;
        }
        let is_property = line.split_once(": ").is_some_and(|(key, _)| {
            !key.is_empty()
                && key.len() <= 100
                && !key.starts_with(['#', '-', '*', '>', '|'])
        });
        if !is_property {
            break;
        }
        let _ = lines.next();
    }
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stripped_from_paths() {
        assert_eq!(
            strip_path(
                "Engineering 0123456789abcdef0123456789abcdef/Gateway \
                    fedcba9876543210fedcba9876543210.md"
            ),
            "Engineering/Gateway.md"
        );
        assert_eq!(strip_id("Plain name"), "Plain name");
    }

    #[test]
    fn row_properties_are_dropped_from_bodies() {
        let text = "# Rotate keys\n\nStatus: Done\nOwner: Priya\n\n\
            Rotate the signing keys.\n";
        assert_eq!(strip_properties(text), "\nRotate the signing keys.\n");
    }
}
