// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask help`: checks the help books in `docs/*.book`, and with
//! `write`, regenerates what is derived from them.
//!
//! Derived files: the golden corpus inputs in `corpus/guide/` and the page
//! table that embeds the books in `vellum-help`, and the app's list of
//! help IDs in `web/src/help/topics.gen.ts`. `write` also gives new
//! pages an ID and records it in `book.yaml`.

mod book;
mod coverage;
mod examples;
mod generate;
mod i18n;
mod markdown;
mod ulid;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use book::{Book, Manifest, Node};
use examples::Example;

/// Where the derived corpus inputs go.
const CORPUS_DIR: &str = "corpus/guide";
/// The generated page table.
const PAGES_RS: &str = "crates/vellum-help/src/pages.rs";
/// The app's list of contextual help IDs.
const TOPICS_TS: &str = "web/src/help/topics.gen.ts";
/// The contextual help map.
const ANCHORS: &str = "docs/help-anchors.yaml";

/// Runs `cargo xtask help [write]`.
pub fn run(mode: Option<&str>) -> Result<(), String> {
    let write = match mode {
        None | Some("check") => false,
        Some("write") => true,
        Some(other) => {
            return Err(format!(
                "unknown mode `{other}`; usage: cargo xtask help [check|write]"
            ));
        }
    };
    let root = workspace_root()?;
    if write {
        assign_ids(&root)?;
    }
    let books = load_books(&root)?;
    let mut errors = Vec::new();
    let examples = check_books(&books, &mut errors);
    let anchors = check_anchors(&root, &books, &mut errors)?;
    i18n::check(&root, &books, &mut errors)?;
    let mut outputs = generate::corpus(&examples);
    let _ = outputs.insert(
        PathBuf::from(PAGES_RS),
        generate::page_table(&books, &anchors),
    );
    let _ =
        outputs.insert(PathBuf::from(TOPICS_TS), generate::topics_ts(&anchors));
    sync_outputs(&root, &outputs, write, &mut errors)?;
    if errors.is_empty() {
        Ok(())
    } else {
        errors.push(String::from(
            "help books: fix the problems above; `cargo xtask help write` \
                regenerates derived files",
        ));
        Err(errors.join("\n"))
    }
}

/// The workspace root: the parent of the `xtask` crate.
fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("xtask has no parent folder"))
}

/// Loads every `docs/*.book`, sorted by name.
fn load_books(root: &Path) -> Result<Vec<Book>, String> {
    let docs = root.join("docs");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&docs)
        .map_err(|e| format!("cannot list {}: {e}", docs.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_some_and(|x| x == "book"))
        .collect();
    dirs.sort();
    dirs.iter().map(|d| Book::load(d)).collect()
}

/// Checks structure, IDs, examples and coverage; returns the examples.
fn check_books(books: &[Book], errors: &mut Vec<String>) -> Vec<Example> {
    let mut seen_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut found: Vec<Example> = Vec::new();
    for book in books {
        let name = &book.name;
        check_id(
            &mut seen_ids,
            book.manifest.id.as_deref(),
            &format!("docs/{name}.book/book.yaml"),
            errors,
        );
        for entry in &book.entries {
            if let Some(path) = &entry.path
                && book.page(path).is_none()
            {
                errors.push(format!(
                    "docs/{name}.book/SUMMARY.md: `{path}` does not exist"
                ));
            }
        }
        for path in &book.unlisted {
            errors.push(format!(
                "docs/{name}.book/{path}: not listed in SUMMARY.md"
            ));
        }
        let mut node_ids = BTreeSet::new();
        for page in &book.pages {
            let location = format!("docs/{name}.book/{}", page.path);
            let id = page.front.as_ref().and_then(|f| f.id.as_deref());
            check_id(&mut seen_ids, id, &location, errors);
            if page.front.as_ref().and_then(|f| f.title.as_ref()).is_none() {
                errors.push(format!("{location}: front matter needs a title"));
            }
            if let Some(id) = id {
                let _ = node_ids.insert(id.to_owned());
                let node = book.manifest.nodes.iter().find(|n| n.id == id);
                if node.and_then(|n| n.path.as_deref())
                    != Some(page.path.as_str())
                {
                    errors.push(format!(
                        "{location}: book.yaml has no node `{id}` with this \
                            path"
                    ));
                }
            }
            check_mermaid(&location, &page.text, errors);
            found.extend(examples::collect(&location, &page.text, errors));
        }
        for node in &book.manifest.nodes {
            if !node_ids.contains(&node.id) {
                errors.push(format!(
                    "docs/{name}.book/book.yaml: node `{}` is not a page in \
                        SUMMARY.md",
                    node.id
                ));
            }
        }
    }
    let mut names = BTreeSet::new();
    for example in &found {
        if !names.insert(example.name.as_str()) {
            errors.push(format!(
                "example `{}` appears more than once",
                example.name
            ));
        }
    }
    for required in coverage::REQUIRED {
        if !names.contains(required) {
            errors.push(format!(
                "the user guide has no example `{required}` (R-HG-2)"
            ));
        }
    }
    found
}

/// Checks that an ID exists, is a ULID and is used once.
fn check_id(
    seen: &mut BTreeMap<String, String>,
    id: Option<&str>,
    location: &str,
    errors: &mut Vec<String>,
) {
    let Some(id) = id else {
        errors.push(format!("{location}: missing `id`"));
        return;
    };
    if !ulid::is_valid(id) {
        errors.push(format!("{location}: `{id}` is not a ULID"));
    }
    if let Some(other) = seen.insert(id.to_owned(), location.to_owned()) {
        errors.push(format!("{location}: id `{id}` is also used by {other}"));
    }
}

/// Checks that every `mermaid` fence starts with a known diagram type.
fn check_mermaid(location: &str, text: &str, errors: &mut Vec<String>) {
    for fence in markdown::fences(text) {
        if fence.info.split_whitespace().next() != Some("mermaid") {
            continue;
        }
        let first = fence.body.split_whitespace().next().unwrap_or_default();
        if !coverage::MERMAID_TYPES.contains(&first) {
            errors.push(format!(
                "{location}:{}: `{first}` is not a Mermaid diagram type",
                fence.open_line + 1
            ));
        }
    }
}

/// Checks `docs/help-anchors.yaml`; returns its entries in file order.
fn check_anchors(
    root: &Path,
    books: &[Book],
    errors: &mut Vec<String>,
) -> Result<Vec<(String, String)>, String> {
    let text = book::read(&root.join(ANCHORS))?;
    let mut entries = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let location = format!("{ANCHORS}:{}", index + 1);
        let Some((id, target)) = line.split_once(": ") else {
            errors.push(format!(
                "{location}: expected `help.id: book/page.md#anchor`"
            ));
            continue;
        };
        if !ids.insert(id.to_owned()) {
            errors.push(format!("{location}: `{id}` appears more than once"));
        }
        let (path, fragment) = target
            .split_once('#')
            .map_or((target, None), |(p, f)| (p, Some(f)));
        let page = path.split_once('/').and_then(|(name, page)| {
            books
                .iter()
                .find(|b| b.name == name)
                .and_then(|b| b.page(page))
        });
        match (page, fragment) {
            (None, _) => errors.push(format!(
                "{location}: `{path}` is not a page of a help book"
            )),
            (Some(page), Some(fragment))
                if !markdown::anchors(&page.text)
                    .iter()
                    .any(|a| a == fragment) =>
            {
                errors.push(format!(
                    "{location}: `{path}` has no heading `#{fragment}`"
                ));
            }
            _ => {}
        }
        entries.push((id.to_owned(), target.to_owned()));
    }
    Ok(entries)
}

/// Compares or writes derived files, and removes stale corpus inputs.
fn sync_outputs(
    root: &Path,
    outputs: &BTreeMap<PathBuf, String>,
    write: bool,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    for (relative, expected) in outputs {
        let path = root.join(relative);
        let current = fs::read_to_string(&path).ok();
        if current.as_deref() == Some(expected.as_str()) {
            continue;
        }
        if write {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("cannot create {}: {e}", parent.display())
                })?;
            }
            fs::write(&path, expected)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        } else {
            errors.push(format!("{} is out of date", relative.display()));
        }
    }
    let mut existing = Vec::new();
    list_markdown(&root.join(CORPUS_DIR), &mut existing);
    for path in existing {
        let relative = path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        if outputs.contains_key(&relative) {
            continue;
        }
        if write {
            fs::remove_file(&path).map_err(|e| {
                format!("cannot remove {}: {e}", path.display())
            })?;
        } else {
            errors.push(format!(
                "{} has no example in the guide",
                relative.display()
            ));
        }
    }
    Ok(())
}

/// Collects `.md` files under `dir`, if it exists.
fn list_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if path.is_dir() {
            list_markdown(&path, out);
        } else if path.extension().is_some_and(|x| x == "md")
            && !path.to_string_lossy().ends_with(".canonical.md")
        {
            out.push(path);
        }
    }
}

/// Gives every page without an `id` a new ULID, and makes `book.yaml`
/// list every page in `SUMMARY.md` order, keeping each node's other keys.
fn assign_ids(root: &Path) -> Result<(), String> {
    for book in load_books(root)? {
        let mut nodes = Vec::new();
        for page in &book.pages {
            let file = book.dir.join(&page.path);
            let id = if let Some(id) =
                page.front.as_ref().and_then(|f| f.id.clone())
            {
                id
            } else {
                let id =
                    ulid::generate(&format!("{}/{}", book.name, page.path));
                let title = book
                    .entries
                    .iter()
                    .find(|e| e.path.as_deref() == Some(page.path.as_str()))
                    .map_or("Untitled", |e| e.title.as_str());
                let text = page.text.strip_prefix("---\n").map_or_else(
                    || {
                        format!(
                            "---\nid: {id}\ntitle: {title}\nvellum: 1\n---\n\n\
                                {}",
                            page.text
                        )
                    },
                    |rest| format!("---\nid: {id}\n{rest}"),
                );
                fs::write(&file, text).map_err(|e| {
                    format!("cannot write {}: {e}", file.display())
                })?;
                id
            };
            let old = book.manifest.nodes.iter().find(|n| n.id == id);
            nodes.push(Node {
                id,
                path: Some(page.path.clone()),
                extra: old.map(|n| n.extra.clone()).unwrap_or_default(),
            });
        }
        let manifest = Manifest {
            nodes,
            ..book.manifest.clone()
        };
        let path = book.dir.join("book.yaml");
        fs::write(&path, book::render_manifest(&manifest))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(())
}
