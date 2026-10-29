// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum reindex`: rebuild the derived index tables from the files (D48).
//!
//! The scan reads the workspace into memory, then one transaction replaces
//! spaces, folders, documents and `page_ancestors`. Grants and per-user
//! state are left alone. Readers see the old index until the commit.

mod scan;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;
use vellum_kernel::ObjectId;

use crate::error::{Error, Result};
use crate::index::{Index, Pool};
use crate::telemetry::traced;

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

#[derive(Debug)]
struct SpaceRow {
    space_id: ObjectId,
    kind: SpaceKind,
    slug: String,
    root_path: String,
    name: String,
    plaintext: bool,
}

#[derive(Debug)]
struct FolderRow {
    space_id: ObjectId,
    folder_id: ObjectId,
    parent_id: Option<ObjectId>,
    path: String,
    name: String,
    icon: Option<String>,
}

#[derive(Debug)]
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
struct Snapshot {
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

fn now_ms() -> i64 {
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
        let mut tx = $pool.begin().await?;
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
        let root_owned: PathBuf = root.to_path_buf();
        let snap = tokio::task::spawn_blocking(move || {
            scan::Scanner::new(&root_owned, &reader, now).run()
        })
        .await
        .map_err(|err| Error::Io(io::Error::other(err)))?;
        let system = self.backend().otel_system();
        match &self.pool {
            Pool::Sqlite(pool) => write_snapshot!(pool, system, &snap, now),
            Pool::Postgres(pool) => write_snapshot!(pool, system, &snap, now),
        }
        let _ = tracing::Span::current().record("documents", snap.documents.len());
        for problem in &snap.problems {
            tracing::warn!(%problem, "reindex skipped a path");
        }
        Ok(ReindexReport {
            spaces: snap.spaces.len(),
            folders: snap.folders.len(),
            documents: snap.documents.len(),
            problems: snap.problems,
        })
    }
}
