// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Book folders in the blob store: finding, loading, saving, snapshots.
//!
//! A load reads `book.yaml`, `SUMMARY.md` and every Markdown file under
//! the `.book` folder (never `.snapshots/` or `.mdbook/`), remembering
//! each file's etag. A save writes only what changed: chapters first,
//! then `SUMMARY.md`, then `book.yaml`, each guarded by the etag it was
//! read at, so an edit made meanwhile (in the app or on disk) fails the
//! save with a conflict instead of being overwritten.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use bytes::Bytes;
use futures::TryStreamExt as _;
use vellum_books::fs::{MDBOOK_BUILD_DIR, MDBOOK_TOML};
use vellum_books::manifest::{self, Manifest};
use vellum_books::publish::mdbook_toml;
use vellum_books::snapshot::{self, Snapshot};
use vellum_books::summary;
use vellum_books::{Book, BookError, IdSource, NodeId, SequentialIds};
use vellum_kernel::{BookId, Error, ErrorKind, ObjectId};
use vellum_store::layout::Kind;
use vellum_store::{BlobPath, BlobStore, Etag, Precondition, StoreError};

/// Largest file read as a chapter, manifest or snapshot, in bytes.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Most files one book folder may hold.
pub const MAX_FILES: usize = 20_000;

/// Maps a store failure onto an API error.
#[must_use]
pub fn store_error(e: StoreError) -> Error {
    let kind = match &e {
        StoreError::NotFound(_) => ErrorKind::NotFound,
        StoreError::AlreadyExists(_)
        | StoreError::PreconditionFailed { .. } => ErrorKind::Conflict,
        StoreError::InvalidPath { .. } => ErrorKind::InvalidInput,
        _ => ErrorKind::Unavailable,
    };
    let message = if kind == ErrorKind::Conflict {
        "the book changed meanwhile; reload it and try again"
    } else {
        "the book files could not be read or written"
    };
    Error::new(kind, message).with_source(e)
}

/// Maps a book failure onto an API error.
#[must_use]
pub fn book_error(e: BookError) -> Error {
    let kind = match &e {
        BookError::UnknownNode(_) => ErrorKind::NotFound,
        BookError::Snapshot(_)
        | BookError::Render { .. }
        | BookError::Io { .. } => ErrorKind::Internal,
        _ => ErrorKind::InvalidInput,
    };
    let message = e.to_string();
    Error::new(kind, message).with_source(e)
}

fn record(op: &'static str, started: Instant, files: usize) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        histogram.vellum.books.store_ms = ms,
        books.op = op,
        books.files = files,
        "book store io"
    );
}

/// Mints section IDs with the kernel's generator.
#[must_use]
pub fn fresh_ids() -> impl IdSource {
    let mut fallback = SequentialIds::default();
    move || {
        NodeId::parse(&ObjectId::generate().to_string())
            .unwrap_or_else(|_| fallback.next_id())
    }
}

/// `path` relative to `dir`, if it lies beneath it.
fn relative<'a>(dir: &BlobPath, path: &'a BlobPath) -> Option<&'a str> {
    path.as_str()
        .strip_prefix(dir.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
}

async fn read_text(
    store: &dyn BlobStore,
    path: &BlobPath,
) -> Result<(String, Etag), Error> {
    let blob = store.get(path).await.map_err(store_error)?;
    Ok((
        String::from_utf8_lossy(&blob.bytes).into_owned(),
        blob.meta.etag,
    ))
}

/// Every book in the store, by ID. Folders whose `book.yaml` is missing
/// an ID or malformed are skipped (the load reports why).
///
/// # Errors
///
/// When the store cannot be listed.
pub async fn locate(
    store: &dyn BlobStore,
) -> Result<BTreeMap<BookId, BlobPath>, Error> {
    let started = Instant::now();
    let entries: Vec<_> =
        store.list(None).try_collect().await.map_err(store_error)?;
    let mut out = BTreeMap::new();
    for entry in entries {
        let Kind::BookManifest { book } = Kind::of(&entry.path) else {
            continue;
        };
        if entry.path.file_name() != manifest::FILE_NAME
            || entry.size > MAX_FILE_BYTES
        {
            continue;
        }
        let Ok((text, _)) = read_text(store, &entry.path).await else {
            continue;
        };
        if let Some(id) = Manifest::parse(&text)
            .ok()
            .and_then(|m| m.id.as_str().parse::<BookId>().ok())
        {
            let _ = out.insert(id, book);
        } else {
            tracing::warn!(book.dir = %book, "book.yaml has no valid id; \
                skipped");
        }
    }
    record("locate", started, out.len());
    Ok(out)
}

/// A book read from the store, with what each file held when read.
#[derive(Debug)]
pub struct Loaded {
    /// The `.book` folder.
    pub dir: BlobPath,
    /// The book, with outside edits repaired in memory.
    pub book: Book,
    /// How many repairs the load made (they are saved with the next edit).
    pub repairs: usize,
    read: BTreeMap<String, (String, Etag)>,
}

fn is_book_text(relative: &str) -> bool {
    let hidden = relative.starts_with(snapshot::DIR)
        || relative.starts_with(MDBOOK_BUILD_DIR)
        || relative.split('/').any(|s| s.starts_with('.'));
    !hidden
        && (relative == manifest::FILE_NAME
            || relative == summary::FILE_NAME
            || std::path::Path::new(relative)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md")))
}

/// Loads the book in `dir`.
///
/// # Errors
///
/// `not_found` without `book.yaml` or `SUMMARY.md`, `invalid_input` for
/// a malformed book, or a store failure.
pub async fn load(
    store: &dyn BlobStore,
    dir: &BlobPath,
) -> Result<Loaded, Error> {
    let started = Instant::now();
    let entries: Vec<_> = store
        .list(Some(dir))
        .try_collect()
        .await
        .map_err(store_error)?;
    if entries.len() > MAX_FILES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("the book holds more than {MAX_FILES} files"),
        ));
    }
    let mut read = BTreeMap::new();
    for entry in entries {
        let Some(rel) = relative(dir, &entry.path) else {
            continue;
        };
        if !is_book_text(rel) || entry.size > MAX_FILE_BYTES {
            continue;
        }
        match read_text(store, &entry.path).await {
            Ok(text) => {
                let _ = read.insert(rel.to_owned(), text);
            }
            // Deleted between the listing and the read.
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    let missing = |name: &str| {
        Error::new(ErrorKind::NotFound, format!("the book has no {name}"))
    };
    let manifest = read
        .get(manifest::FILE_NAME)
        .ok_or_else(|| missing(manifest::FILE_NAME))?
        .0
        .clone();
    let summary = read
        .get(summary::FILE_NAME)
        .ok_or_else(|| missing(summary::FILE_NAME))?
        .0
        .clone();
    let mut ids = fresh_ids();
    let (book, repairs) = Book::load(
        &manifest,
        &summary,
        |p| read.get(p.as_str()).map(|(text, _)| text.clone()),
        &mut ids,
    )
    .map_err(book_error)?;
    record("load", started, read.len());
    Ok(Loaded {
        dir: dir.clone(),
        book,
        repairs: repairs.len(),
        read,
    })
}

impl Loaded {
    fn path(&self, relative: &str) -> Result<BlobPath, Error> {
        self.dir.join(relative).map_err(store_error)
    }

    fn condition(&self, relative: &str) -> Precondition {
        self.read
            .get(relative)
            .map_or(Precondition::IfAbsent, |(_, etag)| {
                Precondition::IfMatch(etag.clone())
            })
    }

    /// Writes what changed since the load.
    ///
    /// # Errors
    ///
    /// `conflict` when a file changed meanwhile, or a store failure.
    pub async fn save(
        &mut self,
        store: &dyn BlobStore,
    ) -> Result<usize, Error> {
        let started = Instant::now();
        let mut writes: Vec<(String, String)> = self
            .book
            .files()
            .iter()
            .map(|(path, text)| (path.as_str().to_owned(), text.clone()))
            .collect();
        writes.push((summary::FILE_NAME.to_owned(), self.book.summary_text()));
        writes.push((
            manifest::FILE_NAME.to_owned(),
            self.book.manifest_text().map_err(book_error)?,
        ));
        // Check the manifest first: it is what concurrent structure edits
        // change, so a stale one stops the save before anything is written.
        if let Some((_, etag)) = self.read.get(manifest::FILE_NAME) {
            let now = store
                .head(&self.path(manifest::FILE_NAME)?)
                .await
                .map_err(store_error)?;
            if &now.etag != etag {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "the book changed meanwhile; reload it and try again",
                ));
            }
        }
        let mut written = 0;
        for (rel, text) in writes {
            if self.read.get(&rel).is_some_and(|(old, _)| old == &text) {
                continue;
            }
            let meta = store
                .put(
                    &self.path(&rel)?,
                    Bytes::from(text.clone()),
                    self.condition(&rel),
                )
                .await
                .map_err(store_error)?;
            let _ = self.read.insert(rel, (text, meta.etag));
            written += 1;
        }
        let toml = mdbook_toml(self.book.title(), ".", MDBOOK_BUILD_DIR);
        let _ = store
            .put(
                &self.path(MDBOOK_TOML)?,
                Bytes::from(toml),
                Precondition::None,
            )
            .await
            .map_err(store_error)?;
        let gone: BTreeSet<String> = self
            .book
            .moved_away()
            .iter()
            .chain(self.book.removed())
            .filter(|p| !self.book.files().contains_key(*p))
            .map(|p| p.as_str().to_owned())
            .collect();
        for rel in gone {
            match store.delete(&self.path(&rel)?, Precondition::None).await {
                Ok(()) | Err(StoreError::NotFound(_)) => {}
                Err(e) => return Err(store_error(e)),
            }
            let _ = self.read.remove(&rel);
        }
        self.book.mark_saved();
        record("save", started, written);
        Ok(written)
    }
}

/// The snapshot tags of the book in `dir`.
///
/// # Errors
///
/// When the store cannot be listed.
pub async fn tags(
    store: &dyn BlobStore,
    dir: &BlobPath,
) -> Result<BTreeSet<String>, Error> {
    let snapshots = dir.join(snapshot::DIR).map_err(store_error)?;
    let entries: Vec<_> = store
        .list(Some(&snapshots))
        .try_collect()
        .await
        .map_err(store_error)?;
    Ok(entries
        .iter()
        .filter(|e| e.path.parent().as_ref() == Some(&snapshots))
        .filter_map(|e| e.path.file_name().strip_suffix(".json"))
        .filter(|tag| snapshot::validate_tag(tag).is_ok())
        .map(str::to_owned)
        .collect())
}

/// Reads one snapshot.
///
/// # Errors
///
/// `not_found` for an unknown tag, `invalid_input` for a malformed one.
pub async fn read_snapshot(
    store: &dyn BlobStore,
    dir: &BlobPath,
    tag: &str,
) -> Result<Snapshot, Error> {
    let started = Instant::now();
    let path = dir
        .join(&snapshot::file_name(tag).map_err(book_error)?)
        .map_err(store_error)?;
    let meta = store.head(&path).await.map_err(|e| match e {
        StoreError::NotFound(_) => {
            Error::new(ErrorKind::NotFound, format!("no snapshot {tag:?}"))
        }
        other => store_error(other),
    })?;
    if meta.size > MAX_FILE_BYTES * 8 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the snapshot is too large",
        ));
    }
    let (text, _) = read_text(store, &path).await?;
    let snapshot = Snapshot::from_json(&text).map_err(book_error)?;
    record("snapshot.read", started, snapshot.files.len());
    Ok(snapshot)
}

/// Stores a new snapshot. Tags are immutable.
///
/// # Errors
///
/// `conflict` when the tag exists, or a store failure.
pub async fn write_snapshot(
    store: &dyn BlobStore,
    dir: &BlobPath,
    snapshot: &Snapshot,
) -> Result<(), Error> {
    let started = Instant::now();
    let path = dir
        .join(&snapshot::file_name(&snapshot.tag).map_err(book_error)?)
        .map_err(store_error)?;
    let json = snapshot.to_json().map_err(book_error)?;
    match store
        .put(&path, Bytes::from(json), Precondition::IfAbsent)
        .await
    {
        Ok(_) => {}
        Err(
            StoreError::AlreadyExists(_)
            | StoreError::PreconditionFailed { .. },
        ) => {
            return Err(Error::new(
                ErrorKind::Conflict,
                format!(
                    "snapshot {:?} exists; tags are immutable",
                    snapshot.tag
                ),
            ));
        }
        Err(e) => return Err(store_error(e)),
    }
    record("snapshot.write", started, snapshot.files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_book_text_is_read() {
        assert!(is_book_text("book.yaml"));
        assert!(is_book_text("SUMMARY.md"));
        assert!(is_book_text("onboarding/first-week.md"));
        assert!(is_book_text("README.MD"));
        assert!(!is_book_text(".snapshots/v1.json"));
        assert!(!is_book_text(".mdbook/index.md"));
        assert!(!is_book_text("images/logo.png"));
        assert!(!is_book_text("notes/.draft.md"));
        assert!(!is_book_text("book.toml"));
    }
}
