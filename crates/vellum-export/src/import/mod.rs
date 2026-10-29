// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Import from Word, Google Docs, Markdown folders (and Obsidian vaults),
//! Notion, Confluence, Outline and Docmost.
//!
//! Every importer reads its source into pages, attachments and tables;
//! the shared core then
//!
//! - gives every page a fresh document ID (never one from the source, so
//!   an import can never collide with pages already in the workspace);
//! - rewrites links between imported pages to `vellum://doc/<id>`, so they
//!   survive moves and renames, and links to attachments to their new
//!   relative paths;
//! - writes canonical Vellum Markdown with `id`, `title` and `vellum: 1`
//!   front matter;
//! - turns databases (Notion CSV) into collections (`name.collection/`
//!   with `schema.yaml` and one row page per record).
//!
//! The result is an [`ImportSet`]: files to write under the destination
//! folder, plus warnings for anything that did not map. Nothing is
//! written here; the caller stores the files through the store layer.
//! Archives are read with hard limits on entry count and size, and entry
//! names can never leave the destination folder.

mod archive;
mod confluence;
pub mod html;
mod links;
mod markdown;
mod notion;
mod word;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use vellum_collections::csv::import_new;
use vellum_collections::{Collection, Edit, PropertyType, Workspace};
use vellum_format::front_matter::{Map, Value};
use vellum_format::{Document, FrontMatter, Inline};
use vellum_kernel::{CollectionId, DocId};

pub use archive::{MAX_ENTRIES, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES};

use crate::error::{ExportWarning, ImportError};
use crate::gdocs::{AccessToken, Drive};
use crate::metrics;

/// Longest file or folder name written, in bytes (the store's limit).
const MAX_NAME_BYTES: usize = 200;

/// Largest page text parsed, in bytes.
const MAX_PAGE_BYTES: usize = 10 * 1024 * 1024;

/// Where an import comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// A zip of Markdown files and their attachments.
    Markdown,
    /// A zip of an Obsidian vault: `[[wiki links]]` and `![[embeds]]`.
    Obsidian,
    /// An Outline Markdown export, with `:::info` notices.
    Outline,
    /// A Docmost Markdown export, with `:::info` callouts.
    Docmost,
    /// A Notion "Markdown & CSV" export; databases become collections.
    Notion,
    /// A Confluence space exported as HTML.
    Confluence,
    /// One Word document (`.docx`).
    Word,
}

impl Source {
    /// The metrics and error label.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Obsidian => "obsidian",
            Self::Outline => "outline",
            Self::Docmost => "docmost",
            Self::Notion => "notion",
            Self::Confluence => "confluence",
            Self::Word => "docx",
        }
    }
}

/// What an import produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSet {
    /// Files to write, with paths relative to the destination folder.
    pub files: Vec<ImportedFile>,
    /// What could not be imported as it was.
    pub warnings: Vec<ExportWarning>,
}

impl ImportSet {
    /// The imported pages (collection rows not included).
    pub fn pages(&self) -> impl Iterator<Item = &ImportedFile> {
        self.files
            .iter()
            .filter(|f| matches!(f.kind, FileKind::Page { .. }))
    }

    /// The file at `path`.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&ImportedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// One file of an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedFile {
    /// Where to write it, relative to the destination folder.
    pub path: String,
    /// Its contents; pages are UTF-8 Vellum Markdown.
    pub bytes: Vec<u8>,
    /// What it is.
    pub kind: FileKind,
}

impl ImportedFile {
    /// The contents as text (pages and collection files are UTF-8).
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// The kind of an imported file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    /// A page, with its new document ID and title.
    Page {
        /// The document ID in its front matter.
        id: String,
        /// The page title.
        title: String,
    },
    /// An attachment.
    Asset,
    /// A collection's `schema.yaml` or one of its row pages.
    Collection,
}

/// Imports `bytes` read from `source`. `name` is the uploaded file's name,
/// used for the page of a Word import.
///
/// # Errors
///
/// [`ImportError::Format`] when the bytes are not that kind of export,
/// [`ImportError::TooLarge`] past the archive limits.
#[tracing::instrument(name = "import", skip(bytes), fields(source = source.name(), bytes = bytes.len()))]
pub fn import(source: Source, name: &str, bytes: &[u8]) -> Result<ImportSet, ImportError> {
    let started = Instant::now();
    let result = match source {
        Source::Markdown | Source::Obsidian | Source::Outline | Source::Docmost => {
            markdown::import(source, bytes)
        }
        Source::Notion => notion::import(bytes),
        Source::Confluence => confluence::import(bytes),
        Source::Word => word::import(name, bytes),
    };
    let result = result.map(Builder::finish);
    metrics::import_done(
        source.name(),
        started,
        result.as_ref().map(|set| set.pages().count()),
    );
    if let Ok(set) = &result {
        tracing::info!(
            pages = set.pages().count(),
            files = set.files.len(),
            warnings = set.warnings.len(),
            "import read"
        );
    }
    result
}

/// Imports a Google Doc: Drive exports it as Word, which is then read
/// like any Word file.
///
/// # Errors
///
/// [`ImportError::Remote`] when Drive refuses or fails, and the Word
/// import's errors.
#[tracing::instrument(name = "import.gdocs", skip(drive, token))]
pub async fn google_doc(
    drive: &Drive,
    token: &AccessToken,
    file_id: &str,
) -> Result<ImportSet, ImportError> {
    let (name, bytes) = drive.download_docx(token, file_id).await?;
    import(Source::Word, &name, &bytes)
}

/// A page read from the source, before IDs and links are settled.
#[derive(Debug, Clone)]
pub(crate) struct Page {
    /// Its path in the source, for resolving links to it.
    src: String,
    /// Where it is written.
    out: String,
    /// Its title.
    title: String,
    /// Its text as Vellum Markdown.
    text: String,
    /// The collection it is a row of.
    row_of: Option<usize>,
}

#[derive(Debug, Clone)]
struct Asset {
    src: String,
    out: String,
    bytes: Vec<u8>,
}

/// A table that becomes a collection.
#[derive(Debug, Clone)]
struct Table {
    /// The collection folder (`name.collection`).
    out: String,
    title: String,
    csv: String,
}

/// Collects what an importer reads, then settles IDs and links.
pub(crate) struct Builder {
    format: &'static str,
    pages: Vec<Page>,
    assets: Vec<Asset>,
    tables: Vec<Table>,
    warnings: Vec<ExportWarning>,
    used: HashSet<String>,
}

impl Builder {
    pub(crate) fn new(format: &'static str) -> Self {
        Self {
            format,
            pages: Vec::new(),
            assets: Vec::new(),
            tables: Vec::new(),
            warnings: Vec::new(),
            used: HashSet::new(),
        }
    }

    pub(crate) fn warn(&mut self, location: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(ExportWarning::new(location, message));
    }

    /// Adds a page read from `src`, written at `out` (cleaned and made
    /// unique). Returns its index.
    pub(crate) fn page(&mut self, src: &str, out: &str, title: &str, text: String) -> usize {
        let out = self.unique(&clean_path(out));
        let text = if text.len() > MAX_PAGE_BYTES {
            self.warn(src, "the page is over 10 MB; the rest was cut");
            let mut end = MAX_PAGE_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.get(..end).unwrap_or_default().to_owned()
        } else {
            text
        };
        self.pages.push(Page {
            src: src.to_owned(),
            out,
            title: title.trim().to_owned(),
            text,
            row_of: None,
        });
        self.pages.len() - 1
    }

    /// Adds an attachment.
    pub(crate) fn asset(&mut self, src: &str, out: &str, bytes: Vec<u8>) {
        let out = self.unique(&clean_path(out));
        self.assets.push(Asset {
            src: src.to_owned(),
            out,
            bytes,
        });
    }

    /// Adds a table that becomes a collection at `out` (a folder; the
    /// `.collection` suffix is added). Returns its index for
    /// [`Builder::row_of`].
    pub(crate) fn table(&mut self, out: &str, title: &str, csv: String) -> usize {
        let base = clean_path(out);
        let out = self.unique(&format!("{base}.collection"));
        self.tables.push(Table {
            out,
            title: title.to_owned(),
            csv,
        });
        self.tables.len() - 1
    }

    /// Marks page `page` as the row page of table `table`.
    pub(crate) fn row_of(&mut self, page: usize, table: usize) {
        if let Some(page) = self.pages.get_mut(page) {
            page.row_of = Some(table);
        }
    }

    fn unique(&mut self, path: &str) -> String {
        let (stem, ext) = match path.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.contains('/') => {
                (stem.to_owned(), format!(".{ext}"))
            }
            _ => (path.to_owned(), String::new()),
        };
        let mut candidate = path.to_owned();
        let mut n = 1;
        while !self.used.insert(candidate.to_lowercase()) {
            n += 1;
            candidate = format!("{stem} {n}{ext}");
        }
        candidate
    }

    /// Settles IDs, rewrites links and writes the files.
    #[allow(clippy::too_many_lines)] // one pass per stage, in order
    pub(crate) fn finish(mut self) -> ImportSet {
        let ids: Vec<String> = self
            .pages
            .iter()
            .map(|_| DocId::generate().to_string())
            .collect();
        let index = Index::new(&self.pages, &self.assets);
        let mut docs: Vec<Document> = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            let mut doc = vellum_format::parse(&page.text).document;
            let dir = links::parent(&page.src).to_owned();
            let out_dir = links::parent(&page.out).to_owned();
            let mut missing = Vec::new();
            links::inlines_mut(&mut doc.blocks, &mut |inlines| {
                for inline in inlines.iter_mut() {
                    rewrite(
                        inline,
                        &dir,
                        &out_dir,
                        &index,
                        &ids,
                        &self.assets,
                        &mut missing,
                    );
                }
            });
            for url in missing {
                self.warnings.push(ExportWarning::new(
                    page.src.clone(),
                    format!("link target `{url}` is not in the import; kept as written"),
                ));
            }
            docs.push(doc);
        }

        let mut set = ImportSet::default();
        let mut rows: Vec<Vec<(usize, String)>> = vec![Vec::new(); self.tables.len()];
        for (i, (page, mut doc)) in self.pages.iter().zip(docs).enumerate() {
            let id = ids.get(i).cloned().unwrap_or_default();
            if let Some(table) = page.row_of {
                doc.front_matter = None;
                if let Some(rows) = rows.get_mut(table) {
                    rows.push((i, vellum_format::serialize(&doc)));
                }
                continue;
            }
            doc.front_matter = Some(front_matter(doc.front_matter.take(), &id, &page.title));
            set.files.push(ImportedFile {
                path: page.out.clone(),
                bytes: vellum_format::serialize(&doc).into_bytes(),
                kind: FileKind::Page {
                    id,
                    title: page.title.clone(),
                },
            });
        }

        for (table, rows) in self.tables.iter().zip(rows) {
            match collection(table, &rows, &self.pages, &ids) {
                Ok((files, problems)) => {
                    for problem in problems {
                        self.warnings
                            .push(ExportWarning::new(table.out.clone(), problem));
                    }
                    set.files
                        .extend(files.into_iter().map(|(path, text)| ImportedFile {
                            path: format!("{}/{path}", table.out),
                            bytes: text.into_bytes(),
                            kind: FileKind::Collection,
                        }));
                }
                Err(message) => {
                    self.warnings.push(ExportWarning::new(
                        table.out.clone(),
                        format!("not imported as a collection: {message}"),
                    ));
                }
            }
        }

        set.files
            .extend(self.assets.into_iter().map(|asset| ImportedFile {
                path: asset.out,
                bytes: asset.bytes,
                kind: FileKind::Asset,
            }));
        set.warnings = self.warnings;
        tracing::debug!(
            format = self.format,
            files = set.files.len(),
            "import settled"
        );
        set
    }
}

/// Lookups from source paths, file names and titles.
struct Index {
    pages: HashMap<String, usize>,
    page_names: HashMap<String, Vec<usize>>,
    titles: HashMap<String, Vec<usize>>,
    assets: HashMap<String, usize>,
    asset_names: HashMap<String, Vec<usize>>,
}

impl Index {
    fn new(pages: &[Page], assets: &[Asset]) -> Self {
        let mut index = Self {
            pages: HashMap::new(),
            page_names: HashMap::new(),
            titles: HashMap::new(),
            assets: HashMap::new(),
            asset_names: HashMap::new(),
        };
        for (i, page) in pages.iter().enumerate() {
            let _ = index.pages.insert(page.src.to_lowercase(), i);
            let name = file_name(&page.src).to_lowercase();
            index
                .page_names
                .entry(stem(&name).to_owned())
                .or_default()
                .push(i);
            index.page_names.entry(name).or_default().push(i);
            index
                .titles
                .entry(page.title.to_lowercase())
                .or_default()
                .push(i);
        }
        for (i, asset) in assets.iter().enumerate() {
            let _ = index.assets.insert(asset.src.to_lowercase(), i);
            index
                .asset_names
                .entry(file_name(&asset.src).to_lowercase())
                .or_default()
                .push(i);
        }
        index
    }

    fn page(&self, dir: &str, path: &str) -> Option<usize> {
        let joined = links::join(dir, path)?.to_lowercase();
        self.pages
            .get(&joined)
            .or_else(|| self.pages.get(&format!("{joined}.md")))
            .copied()
            .or_else(|| unique(self.page_names.get(file_name(&joined))))
    }

    fn asset(&self, dir: &str, path: &str) -> Option<usize> {
        let joined = links::join(dir, path)?.to_lowercase();
        self.assets
            .get(&joined)
            .copied()
            .or_else(|| unique(self.asset_names.get(file_name(&joined))))
    }

    fn wiki(&self, target: &str) -> Option<usize> {
        let key = target.trim().to_lowercase();
        let key = key.split_once('#').map_or(key.as_str(), |(k, _)| k);
        let name = file_name(key);
        unique(self.titles.get(key))
            .or_else(|| unique(self.page_names.get(name)))
            .or_else(|| unique(self.page_names.get(&format!("{name}.md"))))
    }
}

fn unique(found: Option<&Vec<usize>>) -> Option<usize> {
    match found.map(Vec::as_slice) {
        Some([one]) => Some(*one),
        _ => None,
    }
}

fn rewrite(
    inline: &mut Inline,
    dir: &str,
    out_dir: &str,
    index: &Index,
    ids: &[String],
    assets: &[Asset],
    missing: &mut Vec<String>,
) {
    match inline {
        Inline::Link { url, .. } if !links::is_external(url) => {
            let (path, fragment) = links::split(url);
            if path.is_empty() {
                return;
            }
            if let Some(page) = index.page(dir, &path) {
                let id = ids.get(page).cloned().unwrap_or_default();
                *url = fragment.map_or_else(
                    || format!("vellum://doc/{id}"),
                    |f| format!("vellum://doc/{id}#{f}"),
                );
            } else if let Some(asset) = index.asset(dir, &path).and_then(|a| assets.get(a)) {
                *url = links::relative(out_dir, &asset.out);
            } else {
                missing.push(url.clone());
            }
        }
        Inline::Image { url, .. } if !links::is_external(url) => {
            let (path, _) = links::split(url);
            if let Some(asset) = index.asset(dir, &path).and_then(|a| assets.get(a)) {
                *url = links::relative(out_dir, &asset.out);
            } else {
                missing.push(url.clone());
            }
        }
        Inline::WikiLink { target, label } => {
            if let Some(page) = index.wiki(target) {
                let id = ids.get(page).cloned().unwrap_or_default();
                let text = label.clone().unwrap_or_else(|| target.clone());
                *inline = Inline::Link {
                    url: format!("vellum://doc/{id}"),
                    title: String::new(),
                    content: vec![Inline::Text { text }],
                };
            }
        }
        _ => {}
    }
}

/// Front matter with the new `id`, the title and the dialect version,
/// keeping other fields the source had (such as `tags`).
fn front_matter(existing: Option<FrontMatter>, id: &str, title: &str) -> FrontMatter {
    let mut old = match existing {
        Some(FrontMatter::Fields(map)) => map,
        _ => Map::new(),
    };
    let mut map = Map::new();
    let _ = map.insert("id".into(), Value::String(id.to_owned()));
    let title = old
        .remove("title")
        .and_then(|t| t.as_str().map(str::to_owned))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| title.to_owned());
    if !title.is_empty() {
        let _ = map.insert("title".into(), Value::String(title));
    }
    let _ = old.remove("id");
    let _ = old.remove("vellum");
    for key in vellum_format::front_matter::KNOWN_KEYS {
        if let Some(value) = old.remove(key) {
            let _ = map.insert(key.into(), value);
        }
    }
    for (key, value) in old {
        let _ = map.insert(key, value);
    }
    let _ = map.insert(
        "vellum".into(),
        Value::from(vellum_format::front_matter::DIALECT_VERSION),
    );
    FrontMatter::Fields(map)
}

/// Builds a collection from a table and its row pages.
/// A collection's files as `(path, text)` and the problems found.
type CollectionFiles = (Vec<(String, String)>, Vec<String>);

fn collection(
    table: &Table,
    rows: &[(usize, String)],
    pages: &[Page],
    ids: &[String],
) -> Result<CollectionFiles, String> {
    let id = CollectionId::generate();
    let read = import_new(&table.csv, id, &table.title).map_err(|e| e.to_string())?;
    let mut problems: Vec<String> = read
        .diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.location, d.message))
        .collect();
    let title_key = read
        .schema
        .properties
        .iter()
        .find(|p| matches!(p.kind, PropertyType::Title))
        .map(|p| p.key.clone())
        .unwrap_or_default();
    let mut bodies: HashMap<String, (usize, &str)> = rows
        .iter()
        .filter_map(|(page, body)| {
            let title = pages.get(*page)?.title.to_lowercase();
            Some((title, (*page, body.as_str())))
        })
        .collect();
    let mut workspace = Workspace::new();
    workspace
        .insert(Collection::new(read.schema.clone()))
        .map_err(|e| e.to_string())?;
    for values in &read.rows {
        let title = values
            .get(&title_key)
            .map(|v| format!("{v}").to_lowercase())
            .unwrap_or_default();
        let (doc, body) = bodies
            .remove(&title)
            .map_or((None, String::new()), |(page, body)| {
                (
                    ids.get(page).and_then(|id| id.parse::<DocId>().ok()),
                    body.to_owned(),
                )
            });
        if let Err(error) = workspace.apply(Edit::CreateRow {
            collection: id,
            id: doc,
            values: values.clone(),
            body,
        }) {
            problems.push(format!("row `{title}` skipped: {error}"));
        }
    }
    for (title, _) in bodies {
        problems.push(format!("row page `{title}` has no row in the table"));
    }
    let files = workspace
        .collection(id)
        .ok_or("the collection was not created")?
        .files()
        .map_err(|e| e.to_string())?;
    Ok((files, problems))
}

/// Whether `path` ends in one of `exts` (without dots), ignoring case.
fn has_ext(path: &str, exts: &[&str]) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The last path segment.
fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// A file name without its extension.
fn stem(name: &str) -> &str {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or(name, |(stem, _)| stem)
}

/// Makes each segment of `path` safe on every storage backend.
pub(crate) fn clean_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(clean_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn clean_segment(segment: &str) -> String {
    let mut out: String = segment
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    out = out.trim().trim_end_matches('.').trim().to_owned();
    if out.len() > MAX_NAME_BYTES {
        let (stem, ext) = match out.rsplit_once('.') {
            Some((s, e)) if e.len() <= 10 => (s.to_owned(), format!(".{e}")),
            _ => (out.clone(), String::new()),
        };
        let mut end = MAX_NAME_BYTES - ext.len();
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        out = format!("{}{ext}", stem.get(..end).unwrap_or_default().trim_end());
    }
    if out.is_empty() || out == "." || out == ".." || out.starts_with('.') {
        out = format!("untitled{out}");
    }
    out
}

/// The title of Markdown text: front matter `title`, else the first `#`
/// heading, else `fallback`.
pub(crate) fn markdown_title(text: &str, fallback: &str) -> String {
    let title = crate::lower::doc_title(&vellum_format::parse(text).document);
    let title = title.trim();
    if title.is_empty() {
        fallback.to_owned()
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests;
