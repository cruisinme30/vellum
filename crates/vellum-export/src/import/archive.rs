// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading zip exports safely.
//!
//! Entry names are resolved with the zip crate's `enclosed_name`, so an
//! absolute path or `..` never escapes the import, and every read is
//! capped: the number of entries, each entry's size and the total size,
//! whatever the archive's headers claim.

use std::io::{Cursor, Read};

use crate::error::ImportError;

/// Most entries read from one archive (nested archives included).
pub const MAX_ENTRIES: usize = 20_000;
/// Largest single entry, uncompressed.
pub const MAX_ENTRY_BYTES: u64 = 100 * 1024 * 1024;
/// Largest total of all entries, uncompressed.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One file from an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its path inside the archive, `/`-separated, never absolute and
    /// never containing `..`.
    pub path: String,
    /// Its contents.
    pub bytes: Vec<u8>,
}

/// Reads every file of a zip archive. Nested `.zip` files are expanded in
/// place when `nested` is set (Notion splits large exports that way).
/// Folders, macOS resource forks and `.DS_Store` files are skipped.
///
/// # Errors
///
/// [`ImportError::Format`] when the bytes are not a zip archive, and
/// [`ImportError::TooLarge`] past the limits above.
pub fn read(format: &'static str, bytes: &[u8], nested: bool) -> Result<Vec<Entry>, ImportError> {
    let mut out = Vec::new();
    let mut total = 0u64;
    read_into(format, bytes, nested, "", &mut out, &mut total)?;
    Ok(out)
}

fn read_into(
    format: &'static str,
    bytes: &[u8],
    nested: bool,
    prefix: &str,
    out: &mut Vec<Entry>,
    total: &mut u64,
) -> Result<(), ImportError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ImportError::format(format, format!("not a zip archive ({e})")))?;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|e| ImportError::format(format, e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        let path: Vec<String> = name
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        if path.is_empty()
            || path.iter().any(|p| p == "__MACOSX")
            || path.last().is_some_and(|p| p == ".DS_Store")
        {
            continue;
        }
        let path = if prefix.is_empty() {
            path.join("/")
        } else {
            format!("{prefix}/{}", path.join("/"))
        };
        if out.len() >= MAX_ENTRIES {
            return Err(ImportError::TooLarge(format!(
                "more than {MAX_ENTRIES} files"
            )));
        }
        let mut data = Vec::new();
        let _ = file
            .by_ref()
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)?;
        let size = data.len() as u64;
        if size > MAX_ENTRY_BYTES {
            return Err(ImportError::TooLarge(format!(
                "{path} is over {} MB",
                MAX_ENTRY_BYTES / 1024 / 1024
            )));
        }
        *total += size;
        if *total > MAX_TOTAL_BYTES {
            return Err(ImportError::TooLarge(
                "the archive expands past 2 GB".into(),
            ));
        }
        drop(file);
        if nested && path.to_ascii_lowercase().ends_with(".zip") {
            let stem = path.get(..path.len() - 4).unwrap_or_default().to_owned();
            read_into(format, &data, false, &stem, out, total)?;
        } else {
            out.push(Entry { path, bytes: data });
        }
    }
    Ok(())
}

/// Removes a folder that every entry shares (`SPACE/…` in Confluence,
/// `Export-…/…` in Notion), so imports start at the content.
pub fn strip_common_root(entries: &mut [Entry]) {
    loop {
        let Some(first) = entries.first() else {
            return;
        };
        let Some((root, _)) = first.path.split_once('/') else {
            return;
        };
        let root = format!("{root}/");
        if !entries.iter().all(|e| e.path.starts_with(&root)) {
            return;
        }
        for entry in entries.iter_mut() {
            entry.path = entry.path.get(root.len()..).unwrap_or_default().to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use zip::write::SimpleFileOptions;

    use super::*;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, bytes) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        let _ = zip.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn escaping_names_are_dropped() {
        let bytes = zip_of(&[
            ("../evil.md", b"x"),
            ("/abs.md", b"x"),
            ("ok/page.md", b"y"),
            ("__MACOSX/ok/._page.md", b"z"),
        ]);
        let entries = read("markdown", &bytes, false).unwrap();
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(!paths.iter().any(|p| p.contains("..")), "{paths:?}");
        assert!(paths.contains(&"ok/page.md"));
        assert!(!paths.iter().any(|p| p.contains("__MACOSX")));
    }

    #[test]
    fn nested_archives_expand_and_roots_strip() {
        let inner = zip_of(&[("Page abc.md", b"# Page\n")]);
        let outer = zip_of(&[("Export-1/Part-1.zip", &inner)]);
        let mut entries = read("notion", &outer, true).unwrap();
        strip_common_root(&mut entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "Page abc.md");
    }

    #[test]
    fn not_a_zip_is_a_format_error() {
        assert!(matches!(
            read("notion", b"plain text", false),
            Err(ImportError::Format { .. })
        ));
    }
}
