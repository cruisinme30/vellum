// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum reindex`: rebuild the derived index tables from the files (D48).
//!
//! The scan reads the workspace into memory, then one transaction replaces
//! spaces, folders, documents and `page_ancestors`. Grants and per-user
//! state are left alone. Readers see the old index until the commit.
//!
//! The workspace is a folder on disk, or a [`Listing`] of a bucket that the
//! caller fetched (ADR 0044, D187); both go through the same scanner, so
//! they give the same rows.

mod scan;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;
use vellum_kernel::ObjectId;

use crate::error::{Error, Result};
use crate::index::{Index, Pool};
use crate::telemetry::traced;
use crate::tx::BeginWrite as _;

/// Reads a file's plaintext. The default reads it as is; encrypted
/// workspaces pass a reader that opens the envelope first.
pub trait FileReader: Send + Sync {
    /// Returns the file's plaintext bytes.
    ///
    /// # Errors
    /// Any I/O or decryption failure; the file is then reported, not indexed.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

/// Reads files straight from disk (plaintext workspaces and tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainFiles;

impl FileReader for PlainFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}

/// The shape of a workspace: which folders and files exist, and when each
/// file last changed. The scanner reads contents through a [`FileReader`].
pub trait Tree: Send + Sync {
    /// The folders and files directly in `dir`, sorted by path. A missing
    /// folder has none.
    fn entries(&self, dir: &Path) -> Vec<PathBuf>;
    /// Whether `path` is a folder.
    fn is_dir(&self, path: &Path) -> bool;
    /// Whether `path` is a file.
    fn is_file(&self, path: &Path) -> bool;
    /// When the file at `path` last changed, if known.
    fn modified(&self, path: &Path) -> Option<SystemTime>;
}

/// A folder on this machine.
#[derive(Debug, Clone, Copy, Default)]
struct Disk;

impl Tree for Disk {
    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        paths
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn modified(&self, path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }
}

/// A workspace held in memory: the listing of a bucket, with the contents
/// of the files the index reads (see [`Listing::wants_content`]).
///
/// Paths are relative to the workspace root, with `/` separators.
/// Folders are implied by the files below them, as in a bucket.
#[derive(Debug, Default, Clone)]
pub struct Listing {
    files: BTreeMap<PathBuf, Listed>,
    children: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
}

#[derive(Debug, Clone)]
struct Listed {
    modified: SystemTime,
    bytes: Option<Vec<u8>>,
}

impl Listing {
    /// An empty listing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the index needs the contents of the file at `path`: the
    /// Markdown pages and the YAML manifests, outside `.vellum/`.
    /// Attachments and other files are listed but never read.
    #[must_use]
    // Case-sensitive like the scanner, which only reads `.md` and `.yaml`.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    pub fn wants_content(path: &str) -> bool {
        !path.starts_with(".vellum/")
            && path.split('/').all(|segment| !segment.is_empty())
            && (path.ends_with(".md") || path.ends_with(".yaml"))
    }

    /// Adds the file at `path`, with its contents when the index needs
    /// them. A later insert of the same path replaces the earlier one.
    pub fn insert(&mut self, path: &str, modified: SystemTime, bytes: Option<Vec<u8>>) {
        let file = PathBuf::from(path);
        let mut parent = PathBuf::new();
        for component in file.components() {
            let child = parent.join(component);
            let _ = self
                .children
                .entry(parent.clone())
                .or_default()
                .insert(child.clone());
            parent = child;
        }
        let _ = self.files.insert(file, Listed { modified, bytes });
    }

    /// The number of files listed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether nothing is listed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl Tree for Listing {
    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        self.children
            .get(dir)
            .map(|c| c.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.children.contains_key(path) && !self.files.contains_key(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn modified(&self, path: &Path) -> Option<SystemTime> {
        self.files.get(path).map(|f| f.modified)
    }
}

impl FileReader for Listing {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.files.get(path) {
            Some(Listed {
                bytes: Some(bytes), ..
            }) => Ok(bytes.clone()),
            Some(_) => Err(io::Error::other("the contents were not fetched")),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }
}

/// What a reindex found and wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReindexReport {
    /// Spaces indexed.
    pub spaces: usize,
    /// Folders indexed.
    pub folders: usize,
    /// Documents indexed (pages, books, collections, rows, issues, task lists, shortcuts).
    pub documents: usize,
    /// Files and folders that were skipped, and why.
    pub problems: Vec<Problem>,
}

/// A path the reindex skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// Path relative to the workspace root, with `/` separators.
    pub path: String,
    /// Why it was skipped.
    pub kind: ProblemKind,
}

/// Why a path was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProblemKind {
    /// No `id` (front matter, `.vellum-folder.yaml`, `book.yaml` or `schema.yaml`).
    /// The store assigns IDs; a reindex never writes files.
    MissingId,
    /// Another path, earlier in sorted order, already holds this ID.
    DuplicateId(ObjectId),
    /// The file could not be read, decrypted or parsed.
    Unreadable(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ProblemKind::MissingId => write!(f, "{}: no id", self.path),
            ProblemKind::DuplicateId(id) => write!(f, "{}: id {id} is already used", self.path),
            ProblemKind::Unreadable(why) => write!(f, "{}: {why}", self.path),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpaceKind {
    SharedDrive,
    UserDrive,
    Team,
    Tasks,
}

impl SpaceKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SharedDrive => "shared_drive",
            Self::UserDrive => "user_drive",
            Self::Team => "team",
            Self::Tasks => "tasks",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Page,
    Book,
    Collection,
    Row,
    Issue,
    TaskList,
    Shortcut,
}

impl DocKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Book => "book",
            Self::Collection => "collection",
            Self::Row => "row",
            Self::Issue => "issue",
            Self::TaskList => "task_list",
            Self::Shortcut => "shortcut",
        }
    }
}

#[derive(Debug, Clone)]
struct SpaceRow {
    space_id: ObjectId,
    kind: SpaceKind,
    slug: String,
    root_path: String,
    name: String,
    plaintext: bool,
}

#[derive(Debug, Clone)]
struct FolderRow {
    space_id: ObjectId,
    folder_id: ObjectId,
    parent_id: Option<ObjectId>,
    path: String,
    name: String,
    icon: Option<String>,
}

#[derive(Debug, Clone)]
struct DocumentRow {
    space_id: ObjectId,
    doc_id: ObjectId,
    folder_id: Option<ObjectId>,
    kind: DocKind,
    path: String,
    title: String,
    status: Option<String>,
    content_hash: String,
    byte_size: i64,
    modified_at: i64,
}

/// Everything one scan found, ready to write.
#[derive(Debug, Default)]
pub struct Snapshot {
    spaces: Vec<SpaceRow>,
    folders: Vec<FolderRow>,
    documents: Vec<DocumentRow>,
    /// (space, ancestor, descendant, depth)
    ancestors: Vec<(ObjectId, ObjectId, ObjectId, i32)>,
    problems: Vec<Problem>,
}

const fn uuid(id: ObjectId) -> Uuid {
    Uuid::from_u128(id.as_u128())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Writes a snapshot in one transaction. A macro because the two pools
/// have different connection types but identical SQL (`$n` placeholders
/// work on both).
macro_rules! write_snapshot {
    ($pool:expr, $system:expr, $snap:expr, $now:expr) => {{
        let snap: &Snapshot = $snap;
        let mut tx = $pool.begin_write().await?;
        // Deleting spaces cascades to folders, documents and page_ancestors.
        let _ = traced($system, "reindex.clear", sqlx::query("DELETE FROM spaces").execute(&mut *tx)).await?;
        for s in &snap.spaces {
            let q = sqlx::query(
                "INSERT INTO spaces (space_id, kind, slug, root_path, name, plaintext, indexed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(uuid(s.space_id))
            .bind(s.kind.as_str())
            .bind(&s.slug)
            .bind(&s.root_path)
            .bind(&s.name)
            .bind(s.plaintext)
            .bind($now);
            let _ = traced($system, "reindex.insert_space", q.execute(&mut *tx)).await?;
        }
        for f in &snap.folders {
            let q = sqlx::query(
                "INSERT INTO folders (space_id, folder_id, parent_id, path, name, icon, indexed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(uuid(f.space_id))
            .bind(uuid(f.folder_id))
            .bind(f.parent_id.map(uuid))
            .bind(&f.path)
            .bind(&f.name)
            .bind(&f.icon)
            .bind($now);
            let _ = traced($system, "reindex.insert_folder", q.execute(&mut *tx)).await?;
        }
        for d in &snap.documents {
            let q = sqlx::query(
                "INSERT INTO documents (space_id, doc_id, folder_id, kind, path, title, status, \
                 content_hash, byte_size, modified_at, indexed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(uuid(d.space_id))
            .bind(uuid(d.doc_id))
            .bind(d.folder_id.map(uuid))
            .bind(d.kind.as_str())
            .bind(&d.path)
            .bind(&d.title)
            .bind(&d.status)
            .bind(&d.content_hash)
            .bind(d.byte_size)
            .bind(d.modified_at)
            .bind($now);
            let _ = traced($system, "reindex.insert_document", q.execute(&mut *tx)).await?;
        }
        for (space, ancestor, descendant, depth) in &snap.ancestors {
            let q = sqlx::query(
                "INSERT INTO page_ancestors (space_id, ancestor_id, descendant_id, depth) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(uuid(*space))
            .bind(uuid(*ancestor))
            .bind(uuid(*descendant))
            .bind(*depth);
            let _ = traced($system, "reindex.insert_ancestor", q.execute(&mut *tx)).await?;
        }
        let q = sqlx::query(
            "INSERT INTO index_meta (key, value) VALUES ('last_reindex_ms', $1) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind($now.to_string());
        let _ = traced($system, "reindex.stamp", q.execute(&mut *tx)).await?;
        tx.commit().await?;
    }};
}

impl Snapshot {
    /// Keeps only the rows of the spaces `keep` accepts. Problems stay with
    /// the first part, so a split snapshot reports each one once.
    pub fn split_off(&self, keep: impl Fn(ObjectId) -> bool, problems: bool) -> Self {
        Self {
            spaces: self
                .spaces
                .iter()
                .filter(|s| keep(s.space_id))
                .cloned()
                .collect(),
            folders: self
                .folders
                .iter()
                .filter(|f| keep(f.space_id))
                .cloned()
                .collect(),
            documents: self
                .documents
                .iter()
                .filter(|d| keep(d.space_id))
                .cloned()
                .collect(),
            ancestors: self
                .ancestors
                .iter()
                .filter(|a| keep(a.0))
                .copied()
                .collect(),
            problems: if problems {
                self.problems.clone()
            } else {
                Vec::new()
            },
        }
    }

    /// The IDs of the spaces found, in scan order.
    pub fn space_ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.spaces.iter().map(|s| s.space_id)
    }

    pub fn report(self) -> ReindexReport {
        ReindexReport {
            spaces: self.spaces.len(),
            folders: self.folders.len(),
            documents: self.documents.len(),
            problems: self.problems,
        }
    }
}

/// Reads the workspace at `root` into memory, off the async runtime.
pub async fn scan<R>(root: &Path, reader: R, now: i64) -> Result<Snapshot>
where
    R: FileReader + 'static,
{
    let root_owned: PathBuf = root.to_path_buf();
    finish(
        tokio::task::spawn_blocking(move || {
            scan::Scanner::new(&root_owned, &Disk, &reader, now).run()
        })
        .await,
    )
}

/// Scans a [`Listing`] the same way, off the async runtime.
pub async fn scan_listing(listing: Listing, now: i64) -> Result<Snapshot> {
    finish(
        tokio::task::spawn_blocking(move || {
            scan::Scanner::new(Path::new(""), &listing, &listing, now).run()
        })
        .await,
    )
}

fn finish(joined: std::result::Result<Snapshot, tokio::task::JoinError>) -> Result<Snapshot> {
    let snap = joined.map_err(|err| Error::Io(io::Error::other(err)))?;
    for problem in &snap.problems {
        tracing::warn!(%problem, "reindex skipped a path");
    }
    Ok(snap)
}

impl Index {
    /// Rebuilds spaces, folders, documents and `page_ancestors` from the
    /// workspace at `root`, in one transaction. Grants and per-user state
    /// are kept. Paths without an ID, with a duplicate ID or that cannot be
    /// read are skipped and listed in the report.
    ///
    /// # Errors
    /// Fails if the scan task panics or the database write fails; the old
    /// index is then left as it was.
    #[tracing::instrument(name = "db.reindex", skip_all, fields(root = %root.display(), documents))]
    pub async fn reindex<R>(&self, root: &Path, reader: R) -> Result<ReindexReport>
    where
        R: FileReader + 'static,
    {
        let now = now_ms();
        let snap = scan(root, reader, now).await?;
        self.write_snapshot(&snap, now).await?;
        let _ = tracing::Span::current().record("documents", snap.documents.len());
        Ok(snap.report())
    }

    /// Like [`Index::reindex`], from a [`Listing`] of a bucket.
    ///
    /// # Errors
    /// As [`Index::reindex`].
    #[tracing::instrument(name = "db.reindex", skip_all, fields(files = listing.len(), documents))]
    pub async fn reindex_listing(&self, listing: Listing) -> Result<ReindexReport> {
        let now = now_ms();
        let snap = scan_listing(listing, now).await?;
        self.write_snapshot(&snap, now).await?;
        let _ = tracing::Span::current().record("documents", snap.documents.len());
        Ok(snap.report())
    }

    /// Replaces the derived tables with `snap` in one transaction.
    pub(crate) async fn write_snapshot(&self, snap: &Snapshot, now: i64) -> Result<()> {
        let system = self.backend().otel_system();
        match &self.pool {
            Pool::Sqlite(pool) => write_snapshot!(pool, system, snap, now),
            Pool::Postgres(pool) => write_snapshot!(pool, system, snap, now),
        }
        Ok(())
    }
}
