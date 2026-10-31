// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading an object's identity from its bytes.
//!
//! The watcher only needs the `id`, so it reads one line rather than
//! parsing the document: Markdown keeps it in the YAML front matter,
//! YAML objects (`book.yaml`, `.vellum-folder.yaml`, collection schemas,
//! shortcuts) as a top-level key. Full parsing belongs to `vellum-format`.

use vellum_kernel::ObjectId;

use crate::{layout::Kind, path::BlobPath};

/// Front matter (or a YAML object) larger than this is not searched.
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// The object id stored in the blob at `path`, if it has one.
#[must_use]
pub fn identity(path: &BlobPath, bytes: &[u8]) -> Option<ObjectId> {
    let head = bytes
        .get(..bytes.len().min(MAX_HEADER_BYTES))
        .unwrap_or_default();
    let text = String::from_utf8_lossy(head);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut lines = text.lines();
    match Kind::of(path) {
        Kind::Document | Kind::Issue { .. } => {
            if lines.next()?.trim_end() != "---" {
                return None;
            }
            let mut front =
                lines.take_while(|l| !matches!(l.trim_end(), "---" | "..."));
            front.find_map(id_value)
        }
        Kind::FolderMeta | Kind::CollectionSchema { .. } | Kind::Shortcut => {
            lines.find_map(id_value)
        }
        Kind::BookManifest { .. } if path.file_name() == "book.yaml" => {
            lines.find_map(id_value)
        }
        _ => None,
    }
}

/// Parses a top-level `id: <ULID>` line; indented keys belong to nested maps.
fn id_value(line: &str) -> Option<ObjectId> {
    let value = line.strip_prefix("id:")?.trim();
    let value = value.split(" #").next().unwrap_or(value).trim();
    let value = value.trim_matches(|c| c == '"' || c == '\'');
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01JBQ3Z8V9G6Q2W4E5R7T8Y9XA";

    fn id_of(path: &str, text: &str) -> Option<String> {
        let path = BlobPath::new(path).ok()?;
        identity(&path, text.as_bytes()).map(|id| id.to_string())
    }

    #[test]
    fn reads_markdown_front_matter() {
        let doc = format!(
            "---\ntitle: Plan\nid: \"{ID}\" # immutable\n---\n# Plan\n"
        );
        assert_eq!(
            id_of("drives/users/ana/Plan.md", &doc).as_deref(),
            Some(ID)
        );
        let crlf = format!("\u{feff}---\r\nid: {ID}\r\n---\r\n");
        assert_eq!(
            id_of("teams/ENG/issues/ENG-1.md", &crlf).as_deref(),
            Some(ID)
        );
    }

    #[test]
    fn ignores_ids_outside_front_matter() {
        assert_eq!(id_of("a.md", &format!("# Plan\n\nid: {ID}\n")), None);
        assert_eq!(
            id_of("a.md", &format!("---\ntitle: x\n---\nid: {ID}\n")),
            None
        );
        assert_eq!(
            id_of("a.md", &format!("---\nowner:\n  id: {ID}\n---\n")),
            None
        );
        assert_eq!(id_of("a.md", "---\nid: not-a-ulid\n---\n"), None);
    }

    #[test]
    fn reads_yaml_objects() {
        let yaml = format!("title: Guide\nid: {ID}\n");
        assert_eq!(
            id_of("drives/shared/Ops/Guide.book/book.yaml", &yaml).as_deref(),
            Some(ID)
        );
        assert_eq!(
            id_of("drives/shared/Ops/.vellum-folder.yaml", &yaml).as_deref(),
            Some(ID)
        );
        assert_eq!(
            id_of("drives/shared/Ops/Guide.book/SUMMARY.md", &yaml),
            None
        );
        assert_eq!(id_of("drives/shared/Ops/logo.png", &yaml), None);
    }
}
