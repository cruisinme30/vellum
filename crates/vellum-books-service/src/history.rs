// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The manifest's own history (R-BK-2, D235).
//!
//! Every edit that changes `book.yaml` or `SUMMARY.md` (a reorder, a nest,
//! an added, renamed or removed section, new section access) leaves one
//! record in `<book>/.history/<id>.json`: who, when, which operation, both
//! files as they were after the change, and the outline that results.
//! Record IDs are time-ordered, so the newest sort last. The folder keeps
//! the newest [`KEEP`] records; snapshots are the long-term record.
//!
//! Chapter text has its own history in the store and the editor, so it is
//! not repeated here.

use std::collections::BTreeSet;
use std::time::Instant;

use bytes::Bytes;
use futures::TryStreamExt as _;
use serde::{Deserialize, Serialize};
use vellum_books::Book;
use vellum_books::summary::Section;
use vellum_books::toc::{TocEntry, TocItem};
use vellum_kernel::{Error, ErrorKind, ObjectId};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};

use crate::api::OutlineLine;
use crate::files::{MAX_FILE_BYTES, store_error};

/// The folder inside the book that holds the records.
pub const DIR: &str = ".history";
/// How many records a book keeps.
pub const KEEP: usize = 500;

/// One recorded change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// The format version; always 1.
    pub vellum: u32,
    /// When (RFC 3339).
    pub at: String,
    /// Who (`user:<id>`), empty for the system.
    pub by: String,
    /// The operation.
    pub op: String,
    /// `book.yaml` after the change.
    pub manifest: String,
    /// `SUMMARY.md` after the change.
    pub summary: String,
    /// The outline after the change.
    pub outline: Vec<OutlineLine>,
}

const fn region(section: Section) -> &'static str {
    match section {
        Section::Prefix => "prefix",
        Section::Numbered => "numbered",
        Section::Suffix => "suffix",
    }
}

fn walk(entry: &TocEntry, parent: Option<&str>, out: &mut Vec<OutlineLine>) {
    let Some(id) = &entry.id else {
        // A draft has no file and no identity to restore.
        return;
    };
    let id = id.to_string();
    out.push(OutlineLine {
        id: id.clone(),
        title: entry.title.clone(),
        label: entry.label.clone(),
        depth: entry.depth,
        parent: parent.map(str::to_owned),
        region: region(entry.slot.section).to_owned(),
    });
    for child in &entry.children {
        walk(child, Some(&id), out);
    }
}

/// The book's sections in reading order, with where each sits.
#[must_use]
pub fn outline(book: &Book) -> Vec<OutlineLine> {
    let mut out = Vec::new();
    for item in &book.toc().items {
        if let TocItem::Chapter(entry) = item {
            walk(entry, None, &mut out);
        }
    }
    out
}

fn folder(dir: &BlobPath) -> Result<BlobPath, Error> {
    dir.join(DIR).map_err(store_error)
}

/// Whether `raw` is a well-formed record ID, so it is safe in a path.
#[must_use]
pub fn is_record_id(raw: &str) -> bool {
    raw.parse::<ObjectId>().is_ok()
}

/// The record IDs of the book in `dir`, oldest first.
///
/// # Errors
///
/// When the store cannot be listed.
pub async fn ids(store: &dyn BlobStore, dir: &BlobPath) -> Result<Vec<String>, Error> {
    let folder = folder(dir)?;
    let entries: Vec<_> = match store.list(Some(&folder)).try_collect().await {
        Ok(entries) => entries,
        Err(StoreError::NotFound(_)) => Vec::new(),
        Err(e) => return Err(store_error(e)),
    };
    let mut ids: Vec<String> = entries
        .iter()
        .filter(|e| e.path.parent().as_ref() == Some(&folder) && e.size <= MAX_FILE_BYTES)
        .filter_map(|e| e.path.file_name().strip_suffix(".json"))
        .filter(|id| is_record_id(id))
        .map(str::to_owned)
        .collect();
    ids.sort_by_key(|id| id.parse::<ObjectId>().map_or(0, ObjectId::as_u128));
    Ok(ids)
}

/// Reads one record.
///
/// # Errors
///
/// `not_found` for an unknown or malformed ID.
pub async fn read(store: &dyn BlobStore, dir: &BlobPath, id: &str) -> Result<Record, Error> {
    let missing = || Error::new(ErrorKind::NotFound, "history entry not found");
    if !is_record_id(id) {
        return Err(missing());
    }
    let path = folder(dir)?
        .join(&format!("{id}.json"))
        .map_err(store_error)?;
    let blob = store.get(&path).await.map_err(|e| match e {
        StoreError::NotFound(_) => missing(),
        other => store_error(other),
    })?;
    serde_json::from_slice::<Record>(&blob.bytes)
        .ok()
        .filter(|r| r.vellum == 1)
        .ok_or_else(missing)
}

/// Appends a record and drops the oldest beyond [`KEEP`].
///
/// # Errors
///
/// A store failure.
pub async fn append(store: &dyn BlobStore, dir: &BlobPath, record: &Record) -> Result<(), Error> {
    let started = Instant::now();
    let folder = folder(dir)?;
    let id = ObjectId::generate().to_string();
    let json = serde_json::to_vec_pretty(record)
        .map_err(|e| Error::new(ErrorKind::Internal, "history record").with_source(e))?;
    let _ = store
        .put(
            &folder.join(&format!("{id}.json")).map_err(store_error)?,
            Bytes::from(json),
            Precondition::IfAbsent,
        )
        .await
        .map_err(store_error)?;
    let all = ids(store, dir).await?;
    let extra: BTreeSet<&String> = all.iter().take(all.len().saturating_sub(KEEP)).collect();
    for old in extra {
        let path = folder.join(&format!("{old}.json")).map_err(store_error)?;
        match store.delete(&path, Precondition::None).await {
            Ok(()) | Err(StoreError::NotFound(_)) => {}
            Err(e) => return Err(store_error(e)),
        }
    }
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        histogram.vellum.books.store_ms = ms,
        books.op = "history.append",
        "book store io"
    );
    Ok(())
}
