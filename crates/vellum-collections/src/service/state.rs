// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The loaded collections and the files behind them.
//!
//! Loading scans the store for `*.collection/schema.yaml`, reads each
//! folder's `rows/*.md` and builds one [`Workspace`]. Each row remembers
//! the path and etag it was read from, so a save rewrites the same file
//! and fails rather than overwriting a change made on disk meanwhile.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use bytes::Bytes;
use futures::TryStreamExt as _;
use vellum_kernel::{CollectionId, DocId, Error, ErrorKind};
use vellum_store::layout::{Kind, SYSTEM_DIR};
use vellum_store::{BlobPath, BlobStore, Etag, Precondition, StoreError};

use crate::error::Diagnostic;
use crate::row::{Row, slug};
use crate::schema::Schema;
use crate::workspace::{Changeset, Collection, ROWS_DIR, RowRef, SCHEMA_FILE, Workspace};

/// Most row files one collection folder may hold.
pub const MAX_ROWS_PER_COLLECTION: usize = 100_000;
/// Largest file read as a schema or row page, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// One collection folder on disk.
#[derive(Debug, Clone)]
pub struct Folder {
    /// The `name.collection` directory.
    pub path: BlobPath,
    /// The `schema.yaml` file and the etag it was read or written at.
    pub schema: (BlobPath, Etag),
    /// Each row's file and the etag it was read or written at.
    pub rows: BTreeMap<DocId, (BlobPath, Etag)>,
    /// Problems found while loading.
    pub diagnostics: Vec<Diagnostic>,
}

/// Everything loaded from the store.
#[derive(Debug, Default)]
pub struct State {
    pub ws: Workspace,
    pub folders: BTreeMap<CollectionId, Folder>,
}

pub fn store_error(e: StoreError) -> Error {
    let kind = match &e {
        StoreError::NotFound(_) => ErrorKind::NotFound,
        StoreError::AlreadyExists(_) | StoreError::PreconditionFailed { .. } => ErrorKind::Conflict,
        StoreError::InvalidPath { .. } => ErrorKind::InvalidInput,
        _ => ErrorKind::Unavailable,
    };
    Error::new(kind, "the collection files could not be read or written").with_source(e)
}

async fn read_text(
    store: &dyn BlobStore,
    path: &BlobPath,
    size: u64,
) -> Result<Option<(String, Etag)>, Error> {
    if size > MAX_FILE_BYTES {
        return Ok(None);
    }
    match store.get(path).await {
        Ok(blob) => Ok(Some((
            String::from_utf8_lossy(&blob.bytes).into_owned(),
            blob.meta.etag,
        ))),
        // Deleted between the listing and the read.
        Err(StoreError::NotFound(_)) => Ok(None),
        Err(e) => Err(store_error(e)),
    }
}

/// Whether `path` is a row page directly inside `collection/rows/`.
fn is_row_of(path: &BlobPath, collection: &BlobPath) -> bool {
    path.parent()
        .is_some_and(|dir| dir.file_name() == ROWS_DIR && dir.parent().as_ref() == Some(collection))
        && path.file_name().to_ascii_lowercase().ends_with(".md")
}

impl State {
    /// Reads every collection in the store.
    pub async fn load(store: &dyn BlobStore) -> Result<Self, Error> {
        let started = Instant::now();
        let entries: Vec<_> = store.list(None).try_collect().await.map_err(store_error)?;
        let mut schemas: Vec<(BlobPath, BlobPath, u64, std::time::SystemTime)> = entries
            .iter()
            .filter_map(|e| match Kind::of(&e.path) {
                Kind::CollectionSchema { collection } => {
                    Some((collection, e.path.clone(), e.size, e.modified))
                }
                _ => None,
            })
            .collect();
        // Oldest first: when a folder is copied with its schema, the copy
        // repeats the collection ID and the original keeps it.
        schemas.sort_by(|a, b| a.3.cmp(&b.3).then_with(|| a.0.cmp(&b.0)));
        let mut state = Self::default();
        let mut total_rows = 0;
        for (dir, schema_path, size, _) in schemas {
            let Some((schema_text, schema_etag)) = read_text(store, &schema_path, size).await?
            else {
                continue;
            };
            let schema = match Schema::from_yaml(&schema_text) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(path = %schema_path, error = %e, "skipping a collection with an invalid schema");
                    continue;
                }
            };
            if let Some(first) = state.folders.get_mut(&schema.id) {
                tracing::warn!(path = %dir, first = %first.path, "skipping a collection whose id is already used");
                first.diagnostics.push(Diagnostic::warning(
                    dir.as_str(),
                    "this folder repeats the collection's id and was not loaded; give its schema.yaml a new id",
                ));
                continue;
            }
            let mut row_files: Vec<_> = entries
                .iter()
                .filter(|e| is_row_of(&e.path, &dir))
                .collect();
            row_files.sort_by(|a, b| a.path.cmp(&b.path));
            let mut folder = Folder {
                path: dir.clone(),
                schema: (schema_path.clone(), schema_etag),
                rows: BTreeMap::new(),
                diagnostics: Vec::new(),
            };
            if row_files.len() > MAX_ROWS_PER_COLLECTION {
                folder.diagnostics.push(Diagnostic::error(
                    dir.as_str(),
                    format!("only the first {MAX_ROWS_PER_COLLECTION} rows are loaded"),
                ));
                row_files.truncate(MAX_ROWS_PER_COLLECTION);
            }
            let mut collection = Collection::new(schema);
            for entry in row_files {
                let Some((text, etag)) = read_text(store, &entry.path, entry.size).await? else {
                    folder.diagnostics.push(Diagnostic::error(
                        entry.path.as_str(),
                        "the file is too large or vanished and was skipped",
                    ));
                    continue;
                };
                let (mut row, mut diags) =
                    Row::parse(&text, collection.schema(), entry.path.as_str());
                folder.diagnostics.append(&mut diags);
                if folder.rows.contains_key(&row.id) {
                    // A copied file: keep both rows, but the copy needs its
                    // own identity before it can be saved.
                    let fresh = DocId::generate();
                    folder.diagnostics.push(Diagnostic::warning(
                        entry.path.as_str(),
                        format!("row id {} is used twice; assigned {fresh}", row.id),
                    ));
                    row.id = fresh;
                }
                let _ = folder.rows.insert(row.id, (entry.path.clone(), etag));
                collection.insert_row(row);
            }
            total_rows += collection.len();
            let id = collection.id();
            if let Err(e) = state.ws.insert(collection) {
                tracing::warn!(path = %dir, error = %e, "skipping a collection whose links do not check");
                continue;
            }
            let _ = state.folders.insert(id, folder);
        }
        crate::telemetry::loaded(total_rows, started);
        Ok(state)
    }

    /// The folder of a loaded collection.
    pub fn folder(&self, id: CollectionId) -> Result<&Folder, Error> {
        self.folders
            .get(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))
    }

    /// The collections that an edit of `id` may write to: itself and the
    /// other side of each two-way relation.
    pub fn write_set(&self, id: CollectionId) -> BTreeSet<CollectionId> {
        let mut out = BTreeSet::from([id]);
        if let Some(c) = self.ws.collection(id) {
            for p in &c.schema().properties {
                if let crate::schema::PropertyType::Relation { to, back: Some(_) } = &p.kind {
                    let _ = out.insert(*to);
                }
            }
        }
        out
    }

    /// The collections whose rows a read of `id` may show: itself and
    /// every relation target (titles in relation cells, rollup values).
    pub fn read_set(&self, id: CollectionId) -> BTreeSet<CollectionId> {
        let mut out = BTreeSet::from([id]);
        if let Some(c) = self.ws.collection(id) {
            for p in &c.schema().properties {
                if let crate::schema::PropertyType::Relation { to, .. } = &p.kind {
                    let _ = out.insert(*to);
                }
            }
        }
        out
    }

    /// Writes the files a [`Changeset`] names. Changed rows are rewritten
    /// in place if the file still has the etag it was read at; new rows
    /// get a fresh file named after their title; deleted rows move to
    /// `.vellum/trash/collections/<collection>/<row>.md`.
    pub async fn write(&mut self, store: &dyn BlobStore, changes: &Changeset) -> Result<(), Error> {
        for r in &changes.written {
            self.write_row(store, *r).await?;
        }
        for r in &changes.deleted {
            self.trash_row(store, *r).await?;
        }
        Ok(())
    }

    /// Writes a new collection folder `dir` with every file of
    /// `collection`, then loads it. Fails with `conflict` when the folder
    /// already holds a collection.
    pub async fn create(
        &mut self,
        store: &dyn BlobStore,
        dir: BlobPath,
        collection: Collection,
    ) -> Result<(), Error> {
        let names = collection.file_names();
        let mut rows = BTreeMap::new();
        let mut schema = None;
        let by_name: BTreeMap<&String, DocId> = names.iter().map(|(id, n)| (n, *id)).collect();
        for (rel, text) in collection.files()? {
            let path = dir.join(&rel).map_err(store_error)?;
            let meta = match store
                .put(&path, Bytes::from(text), Precondition::IfAbsent)
                .await
            {
                Ok(meta) => meta,
                Err(StoreError::AlreadyExists(_) | StoreError::PreconditionFailed { .. }) => {
                    return Err(Error::new(
                        ErrorKind::Conflict,
                        format!("{dir} already exists"),
                    ));
                }
                Err(e) => return Err(store_error(e)),
            };
            if let Some(id) = by_name.get(&rel) {
                let _ = rows.insert(*id, (path, meta.etag));
            } else if rel == SCHEMA_FILE {
                schema = Some((path, meta.etag));
            }
        }
        let schema = schema.ok_or_else(|| {
            Error::new(ErrorKind::Internal, "the new collection has no schema.yaml")
        })?;
        let folder = Folder {
            path: dir,
            schema,
            rows,
            diagnostics: Vec::new(),
        };
        let id = collection.id();
        self.ws.insert(collection)?;
        let _ = self.folders.insert(id, folder);
        Ok(())
    }

    /// Rewrites a collection's `schema.yaml` from the loaded schema, if
    /// the file still has the etag it was read at.
    pub async fn write_schema(
        &mut self,
        store: &dyn BlobStore,
        id: CollectionId,
    ) -> Result<(), Error> {
        let text = self
            .ws
            .collection(id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))?
            .schema()
            .to_yaml()?;
        let folder = self
            .folders
            .get_mut(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))?;
        let (path, etag) = folder.schema.clone();
        let meta = store
            .put(&path, Bytes::from(text), Precondition::IfMatch(etag))
            .await
            .map_err(store_error)?;
        folder.schema = (path, meta.etag);
        Ok(())
    }

    async fn write_row(&mut self, store: &dyn BlobStore, r: RowRef) -> Result<(), Error> {
        let (text, title) = {
            let c = self.ws.collection(r.collection).ok_or_else(|| {
                Error::new(ErrorKind::Internal, "written row's collection vanished")
            })?;
            let row = c
                .row(r.row)
                .ok_or_else(|| Error::new(ErrorKind::Internal, "written row vanished"))?;
            (row.to_markdown(c.schema()), row.title(c.schema()))
        };
        let folder = self
            .folders
            .get_mut(&r.collection)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "written row's folder vanished"))?;
        let bytes = Bytes::from(text);
        if let Some((path, etag)) = folder.rows.get(&r.row).cloned() {
            let meta = store
                .put(&path, bytes, Precondition::IfMatch(etag))
                .await
                .map_err(store_error)?;
            let _ = folder.rows.insert(r.row, (path, meta.etag));
            return Ok(());
        }
        let taken: BTreeSet<&str> = folder.rows.values().map(|(p, _)| p.file_name()).collect();
        let base = match slug(&title) {
            s if s.is_empty() => "untitled".to_owned(),
            s => s,
        };
        let candidates = (1..=50)
            .map(|n| {
                if n == 1 {
                    format!("{base}.md")
                } else {
                    format!("{base}-{n}.md")
                }
            })
            .filter(|name| !taken.contains(name.as_str()))
            .collect::<Vec<_>>()
            .into_iter()
            .chain(std::iter::once(
                format!("{base}-{}.md", r.row).to_lowercase(),
            ));
        for name in candidates {
            let path = folder
                .path
                .join(&format!("{ROWS_DIR}/{name}"))
                .map_err(store_error)?;
            match store
                .put(&path, bytes.clone(), Precondition::IfAbsent)
                .await
            {
                Ok(meta) => {
                    let _ = folder.rows.insert(r.row, (path, meta.etag));
                    return Ok(());
                }
                // A file we have not loaded holds the name; try the next.
                Err(StoreError::AlreadyExists(_) | StoreError::PreconditionFailed { .. }) => {}
                Err(e) => return Err(store_error(e)),
            }
        }
        Err(Error::new(
            ErrorKind::Conflict,
            "no free file name for the new row",
        ))
    }

    async fn trash_row(&mut self, store: &dyn BlobStore, r: RowRef) -> Result<(), Error> {
        let Some(folder) = self.folders.get_mut(&r.collection) else {
            return Ok(());
        };
        let Some((path, _)) = folder.rows.remove(&r.row) else {
            return Ok(());
        };
        let to = BlobPath::new(format!(
            "{SYSTEM_DIR}/trash/collections/{}/{}.md",
            r.collection, r.row
        ))
        .map_err(store_error)?;
        match store.rename(&path, &to).await {
            Ok(_) | Err(StoreError::NotFound(_)) => Ok(()),
            // The same row was trashed before (restored, then deleted again).
            Err(StoreError::AlreadyExists(_)) => {
                let again = BlobPath::new(format!(
                    "{SYSTEM_DIR}/trash/collections/{}/{}-{}.md",
                    r.collection,
                    r.row,
                    DocId::generate()
                ))
                .map_err(store_error)?;
                store
                    .rename(&path, &again)
                    .await
                    .map(|_| ())
                    .map_err(store_error)
            }
            Err(e) => Err(store_error(e)),
        }
    }
}
