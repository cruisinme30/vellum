// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Walks a workspace folder and turns the drive layout (brief section 6)
//! into index rows. Pure reading: nothing on disk is changed.
//!
//! ```text
//! drives/shared/<drive>/   shared drive    teams/<KEY>/     team (issues/ holds issues)
//! drives/users/<user>/     My Drive        tasks/<user>/    personal task lists
//! ```
//!
//! Every space root and folder carries `.vellum-folder.yaml` with its `id`.
//! `name.book/` (with `book.yaml`) and `name.collection/` (with
//! `schema.yaml`) are documents whose pages hang below them.
//! `name.shortcut.yaml` is a shortcut. `.vellum/` is never indexed.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use vellum_kernel::ObjectId;

use super::{
    DocKind, DocumentRow, FileReader, FolderRow, Problem, ProblemKind, Snapshot, SpaceKind,
    SpaceRow,
};

const FOLDER_MANIFEST: &str = ".vellum-folder.yaml";

/// Space roots, relative to the workspace root, and the kind each holds.
const SPACE_PARENTS: [(&str, SpaceKind); 4] = [
    ("drives/shared", SpaceKind::SharedDrive),
    ("drives/users", SpaceKind::UserDrive),
    ("teams", SpaceKind::Team),
    ("tasks", SpaceKind::Tasks),
];

/// The few keys the index needs from front matter and manifests.
#[derive(Debug, Default, Deserialize)]
struct Header {
    id: Option<String>,
    title: Option<String>,
    name: Option<String>,
    icon: Option<String>,
    status: Option<String>,
    #[serde(default)]
    plaintext: bool,
}

/// The node a walk is currently below: its ID chain and nearest folder.
#[derive(Debug, Clone)]
struct Parent {
    space: ObjectId,
    kind: SpaceKind,
    /// Ancestors from the space down to this node, inclusive.
    chain: Vec<ObjectId>,
    folder: Option<ObjectId>,
    /// Set inside a book or collection: pages there are its children.
    container: Option<DocKind>,
}

pub(super) struct Scanner<'a> {
    root: &'a Path,
    reader: &'a dyn FileReader,
    now_ms: i64,
    seen: HashSet<ObjectId>,
    out: Snapshot,
}

impl<'a> Scanner<'a> {
    pub(super) fn new(root: &'a Path, reader: &'a dyn FileReader, now_ms: i64) -> Self {
        Self {
            root,
            reader,
            now_ms,
            seen: HashSet::new(),
            out: Snapshot::default(),
        }
    }

    pub(super) fn run(mut self) -> Snapshot {
        for (parent, kind) in SPACE_PARENTS {
            for dir in Self::sorted_entries(&self.root.join(parent)) {
                if dir.is_dir() {
                    self.space(&dir, kind);
                }
            }
        }
        self.out
    }

    fn space(&mut self, dir: &Path, kind: SpaceKind) {
        let Some((id, header)) = self.manifest(dir) else {
            return;
        };
        let slug = file_name(dir);
        self.out.spaces.push(SpaceRow {
            space_id: id,
            kind,
            name: header.name.or(header.title).unwrap_or_else(|| slug.clone()),
            slug,
            root_path: self.relative(dir),
            plaintext: header.plaintext,
        });
        self.out.ancestors.push((id, id, id, 0));
        let parent = Parent {
            space: id,
            kind,
            chain: vec![id],
            folder: None,
            container: None,
        };
        self.walk(dir, dir, &parent);
    }

    fn walk(&mut self, space_root: &Path, dir: &Path, parent: &Parent) {
        for path in Self::sorted_entries(dir) {
            let name = file_name(&path);
            if name.starts_with('.')
                || (parent.container == Some(DocKind::Book) && name == "SUMMARY.md")
            {
                continue;
            }
            let rel = relative_to(space_root, &path);
            if path.is_dir() {
                self.directory(space_root, &path, &name, rel, parent);
            } else if let Some(stem) = name.strip_suffix(".md") {
                let kind = match (parent.container, parent.kind) {
                    (Some(DocKind::Collection), _) => DocKind::Row,
                    (None, SpaceKind::Team) if rel.starts_with("issues/") => DocKind::Issue,
                    (None, SpaceKind::Tasks) => DocKind::TaskList,
                    _ => DocKind::Page,
                };
                let _ = self.document(&path, rel, stem, kind, Header::from_front_matter, parent);
            } else if let Some(stem) = name.strip_suffix(".shortcut.yaml") {
                let _ = self.document(
                    &path,
                    rel,
                    stem,
                    DocKind::Shortcut,
                    Header::from_yaml,
                    parent,
                );
            }
        }
    }

    fn directory(
        &mut self,
        space_root: &Path,
        dir: &Path,
        name: &str,
        rel: String,
        parent: &Parent,
    ) {
        let container = [
            (".book", "book.yaml", DocKind::Book),
            (".collection", "schema.yaml", DocKind::Collection),
        ]
        .into_iter()
        .find_map(|(suffix, manifest, kind)| {
            name.strip_suffix(suffix).map(|stem| (stem, manifest, kind))
        });
        if let Some((stem, manifest, kind)) = container {
            let file = dir.join(manifest);
            if let Some(id) = self.document(&file, rel, stem, kind, Header::from_yaml, parent) {
                let mut inner = parent.clone();
                inner.chain.push(id);
                inner.container = Some(kind);
                self.walk(space_root, dir, &inner);
            }
            return;
        }
        let mut inner = parent.clone();
        let structural =
            parent.kind == SpaceKind::Team && parent.chain.len() == 1 && name == "issues";
        if parent.container.is_some() || structural {
            // Plain grouping (a team's `issues/`, a collection's `rows/`), not a folder.
            self.walk(space_root, dir, &inner);
            return;
        }
        if let Some((id, header)) = self.manifest(dir) {
            self.out.folders.push(FolderRow {
                space_id: parent.space,
                folder_id: id,
                parent_id: parent.folder,
                path: rel,
                name: header
                    .name
                    .or(header.title)
                    .unwrap_or_else(|| name.to_owned()),
                icon: header.icon,
            });
            self.push_ancestors(parent, id);
            inner.chain.push(id);
            inner.folder = Some(id);
        }
        // A folder without a manifest is reported by `manifest` and walked
        // through: its contents hang from the nearest identified ancestor.
        self.walk(space_root, dir, &inner);
    }

    /// Indexes one document whose header lives in `file`. Returns its ID.
    fn document(
        &mut self,
        file: &Path,
        rel: String,
        stem: &str,
        kind: DocKind,
        parse: fn(&[u8]) -> Result<Header, String>,
        parent: &Parent,
    ) -> Option<ObjectId> {
        let bytes = self.read(file)?;
        let header = match parse(&bytes) {
            Ok(header) => header,
            Err(detail) => {
                self.problem(file, ProblemKind::Unreadable(detail));
                return None;
            }
        };
        let id = self.claim(file, header.id.as_deref())?;
        let modified_at = fs::metadata(file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(self.now_ms, |d| {
                i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
            });
        self.out.documents.push(DocumentRow {
            space_id: parent.space,
            doc_id: id,
            folder_id: parent.folder,
            kind,
            path: rel,
            title: header.title.unwrap_or_else(|| stem.to_owned()),
            status: header
                .status
                .filter(|s| ["draft", "in-review", "verified", "outdated"].contains(&s.as_str())),
            content_hash: hex(&Sha256::digest(&bytes)),
            byte_size: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            modified_at,
        });
        self.push_ancestors(parent, id);
        Some(id)
    }

    /// Reads a folder's `.vellum-folder.yaml` and claims its ID.
    fn manifest(&mut self, dir: &Path) -> Option<(ObjectId, Header)> {
        let file = dir.join(FOLDER_MANIFEST);
        if !file.is_file() {
            self.problem(dir, ProblemKind::MissingId);
            return None;
        }
        let bytes = self.read(&file)?;
        match Header::from_yaml(&bytes) {
            Ok(header) => self.claim(dir, header.id.as_deref()).map(|id| (id, header)),
            Err(detail) => {
                self.problem(&file, ProblemKind::Unreadable(detail));
                None
            }
        }
    }

    /// Parses an ID and makes sure no earlier path already holds it.
    fn claim(&mut self, path: &Path, id: Option<&str>) -> Option<ObjectId> {
        let Some(text) = id else {
            self.problem(path, ProblemKind::MissingId);
            return None;
        };
        match text.parse::<ObjectId>() {
            Ok(id) if self.seen.insert(id) => Some(id),
            Ok(id) => {
                self.problem(path, ProblemKind::DuplicateId(id));
                None
            }
            Err(err) => {
                self.problem(path, ProblemKind::Unreadable(err.to_string()));
                None
            }
        }
    }

    fn push_ancestors(&mut self, parent: &Parent, id: ObjectId) {
        let depth = parent.chain.len();
        for (i, ancestor) in parent.chain.iter().enumerate() {
            let d = i32::try_from(depth - i).unwrap_or(i32::MAX);
            self.out.ancestors.push((parent.space, *ancestor, id, d));
        }
        self.out.ancestors.push((parent.space, id, id, 0));
    }

    fn read(&mut self, file: &Path) -> Option<Vec<u8>> {
        match self.reader.read(file) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                self.problem(file, ProblemKind::Unreadable(err.to_string()));
                None
            }
        }
    }

    fn problem(&mut self, path: &Path, kind: ProblemKind) {
        let path = self.relative(path);
        self.out.problems.push(Problem { path, kind });
    }

    fn relative(&self, path: &Path) -> String {
        relative_to(self.root, path)
    }

    fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        paths
    }
}

impl Header {
    fn from_yaml(bytes: &[u8]) -> Result<Self, String> {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self::default());
        }
        serde_yaml_ng::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// Reads the YAML between the opening `---` line and the next `---` line.
    fn from_front_matter(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let Some(rest) = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
        else {
            return Ok(Self::default());
        };
        let end = rest
            .match_indices("---")
            .find(|(i, _)| *i == 0 || rest.as_bytes().get(i - 1) == Some(&b'\n'))
            .map_or(rest.len(), |(i, _)| i);
        Self::from_yaml(rest.get(..end).unwrap_or_default().as_bytes())
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `path` relative to `base`, with `/` separators on every platform.
fn relative_to(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}
