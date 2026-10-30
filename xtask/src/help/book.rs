// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A help book on disk: `SUMMARY.md`, `book.yaml` and its pages.

use std::fs;
use std::path::{Path, PathBuf};

use super::markdown::{self, FrontMatter};

/// One chapter link in `SUMMARY.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The link text.
    pub title: String,
    /// The target path, relative to the book; `None` for a draft.
    pub path: Option<String>,
}

/// One node of `book.yaml`: an ID and the lines that describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The page's ULID.
    pub id: String,
    /// The `path:` value.
    pub path: Option<String>,
    /// Other keys of the node (owners, reviewers), kept verbatim.
    pub extra: Vec<String>,
}

/// The parts of `book.yaml` the checks use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The lines before `nodes:`, kept verbatim.
    pub header: Vec<String>,
    /// The book's `id`.
    pub id: Option<String>,
    /// The nodes, in file order.
    pub nodes: Vec<Node>,
}

/// A page of a book.
#[derive(Debug, Clone)]
pub struct Page {
    /// Path relative to the book folder, with `/` separators.
    pub path: String,
    /// The page's text.
    pub text: String,
    /// Its front matter, if it has any.
    pub front: Option<FrontMatter>,
}

/// A loaded book.
#[derive(Debug, Clone)]
pub struct Book {
    /// The short name: `user` for `docs/user.book`.
    pub name: String,
    /// The book folder.
    pub dir: PathBuf,
    /// The chapters in `SUMMARY.md` order.
    pub entries: Vec<Entry>,
    /// The parsed `book.yaml`.
    pub manifest: Manifest,
    /// Pages listed in `SUMMARY.md` that exist, in order.
    pub pages: Vec<Page>,
    /// Markdown files in the folder that `SUMMARY.md` does not list.
    pub unlisted: Vec<String>,
    /// Files under `assets/` folders, relative to the book.
    pub assets: Vec<String>,
}

impl Book {
    /// Loads `docs/<name>.book`.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".book"))
            .ok_or_else(|| format!("{} is not a .book folder", dir.display()))?
            .to_owned();
        let summary = read(&dir.join("SUMMARY.md"))?;
        let entries = parse_summary(&summary);
        let manifest = parse_manifest(&read(&dir.join("book.yaml"))?);
        let mut pages = Vec::new();
        for path in entries.iter().filter_map(|e| e.path.as_deref()) {
            let file = dir.join(path);
            if file.is_file() {
                let text = read(&file)?;
                let front = markdown::front_matter(&text);
                pages.push(Page {
                    path: path.to_owned(),
                    text,
                    front,
                });
            }
        }
        let mut files = Vec::new();
        walk(dir, dir, &mut files)?;
        files.sort();
        let listed: Vec<&str> =
            entries.iter().filter_map(|e| e.path.as_deref()).collect();
        let unlisted = files
            .iter()
            .filter(|f| {
                Path::new(f.as_str())
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("md"))
                    && f.as_str() != "SUMMARY.md"
            })
            .filter(|f| !listed.contains(&f.as_str()))
            .cloned()
            .collect();
        let assets = files
            .into_iter()
            .filter(|f| f.starts_with("assets/") || f.contains("/assets/"))
            .collect();
        Ok(Self {
            name,
            dir: dir.to_path_buf(),
            entries,
            manifest,
            pages,
            unlisted,
            assets,
        })
    }

    /// The page at `path`, if `SUMMARY.md` lists it.
    #[must_use]
    pub fn page(&self, path: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.path == path)
    }
}

/// Reads a UTF-8 file.
pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Lists files under `dir`, relative to `root`, with `/` separators.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            let parts: Vec<String> = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(parts.join("/"));
        }
    }
    Ok(())
}

/// Reads the chapter links of an mdBook `SUMMARY.md`.
#[must_use]
pub fn parse_summary(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let link = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            let rest = link.strip_prefix('[')?;
            let (title, rest) = rest.split_once("](")?;
            let (target, _) = rest.split_once(')')?;
            Some(Entry {
                title: title.to_owned(),
                path: (!target.is_empty()).then(|| target.to_owned()),
            })
        })
        .collect()
}

/// Reads `book.yaml` in the shape the books service writes: top-level
/// keys, then `nodes:` with one ULID key per page and its fields below.
#[must_use]
pub fn parse_manifest(text: &str) -> Manifest {
    let mut manifest = Manifest::default();
    let mut in_nodes = false;
    for line in text.lines() {
        if !in_nodes {
            if line.trim_end() == "nodes:" || line.starts_with("nodes:") {
                in_nodes = true;
                continue;
            }
            if let Some(id) = line.strip_prefix("id:") {
                manifest.id = Some(markdown::unquote(id.trim()).to_owned());
            }
            manifest.header.push(line.to_owned());
            continue;
        }
        if let Some(key) =
            line.strip_prefix("  ").filter(|l| !l.starts_with(' '))
        {
            if let Some(id) = key.trim_end().strip_suffix(':') {
                manifest.nodes.push(Node {
                    id: markdown::unquote(id).to_owned(),
                    path: None,
                    extra: Vec::new(),
                });
            }
        } else if let Some(field) = line.strip_prefix("    ")
            && let Some(node) = manifest.nodes.last_mut()
        {
            match field.strip_prefix("path:") {
                Some(path) => {
                    node.path = Some(markdown::unquote(path.trim()).to_owned())
                }
                None => node.extra.push(line.to_owned()),
            }
        }
    }
    manifest
}

/// Writes `book.yaml` back: the header verbatim, then the nodes.
#[must_use]
pub fn render_manifest(manifest: &Manifest) -> String {
    let mut out = String::new();
    for line in &manifest.header {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("nodes:\n");
    for node in &manifest.nodes {
        out.push_str("  ");
        out.push_str(&node.id);
        out.push_str(":\n");
        if let Some(path) = &node.path {
            out.push_str("    path: ");
            out.push_str(path);
            out.push('\n');
        }
        for line in &node.extra {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_entries_and_drafts() {
        let summary = "# Summary\n\n[Intro](README.md)\n\n# Part\n\n\
            - [A](a/README.md)\n  - [B](a/b.md)\n  - [Draft]()\n\n---\n\n\
            [Glossary](glossary.md)\n";
        let entries = parse_summary(summary);
        let paths: Vec<Option<&str>> =
            entries.iter().map(|e| e.path.as_deref()).collect();
        assert_eq!(
            paths,
            vec![
                Some("README.md"),
                Some("a/README.md"),
                Some("a/b.md"),
                None,
                Some("glossary.md")
            ]
        );
    }

    #[test]
    fn manifest_round_trips() {
        let yaml = "vellum: 1\nid: 01M63C8QH9FN872JSH5DQNY6E9\n\
            title: User guide\n\
            nodes:\n  01JB3K9Q7Z5N8M2X4V6T0R1Y3W:\n    path: README.md\n    \
            owners: [user:priya]\n  01JB3K9Q7Z5N8M2X4V6T0R1Y3X:\n    path: \
            a.md\n";
        let manifest = parse_manifest(yaml);
        assert_eq!(manifest.id.as_deref(), Some("01M63C8QH9FN872JSH5DQNY6E9"));
        assert_eq!(manifest.nodes.len(), 2);
        assert_eq!(
            manifest.nodes.first().and_then(|n| n.path.as_deref()),
            Some("README.md")
        );
        assert_eq!(render_manifest(&manifest), yaml);
    }
}
