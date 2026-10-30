// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Writing a publication into the store (R-BK-10, D237).
//!
//! A publication lives inside the book folder, hidden from the book
//! itself: `<book>/.published/site/` (static HTML, no scripts) or
//! `<book>/.published/mdbook/` (an mdBook source tree for `mdbook build`
//! or the public docs site), each with a `<format>.json` record next to
//! it. Publishing again replaces the folder. Because it is in the store,
//! a Git-backed space pushes it with everything else, and `vellum serve`
//! serves a public site at `/published/books/<book id>/`.

use std::collections::BTreeSet;
use std::time::Instant;

use bytes::Bytes;
use futures::TryStreamExt as _;
use vellum_books::publish::Site;
use vellum_kernel::{BookId, Error, ErrorKind};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};

use crate::api::{Publication, PublishFormat};
use crate::files::{MAX_FILE_BYTES, store_error};

/// The folder inside the book that holds publications.
pub const DIR: &str = ".published";

/// The folder name of a format.
#[must_use]
pub const fn name(format: PublishFormat) -> &'static str {
    match format {
        PublishFormat::Site => "site",
        PublishFormat::Mdbook => "mdbook",
    }
}

/// Where a public site is served, relative to the app origin.
#[must_use]
pub fn url(book: BookId) -> String {
    format!("/published/books/{book}/")
}

/// The folder a format's files go in.
///
/// # Errors
///
/// An invalid path (never for a located book).
pub fn folder(dir: &BlobPath, format: PublishFormat) -> Result<BlobPath, Error> {
    dir.join(&format!("{DIR}/{}", name(format)))
        .map_err(store_error)
}

fn record_path(dir: &BlobPath, format: PublishFormat) -> Result<BlobPath, Error> {
    dir.join(&format!("{DIR}/{}.json", name(format)))
        .map_err(store_error)
}

/// The record of a format's current publication, if any.
///
/// # Errors
///
/// A store failure.
pub async fn current(
    store: &dyn BlobStore,
    dir: &BlobPath,
    format: PublishFormat,
) -> Result<Option<Publication>, Error> {
    match store.get(&record_path(dir, format)?).await {
        Ok(blob) => Ok(serde_json::from_slice(&blob.bytes).ok()),
        Err(StoreError::NotFound(_)) => Ok(None),
        Err(e) => Err(store_error(e)),
    }
}

/// Writes `site` (plus the assets it names, copied from the book folder)
/// into the format's folder, removes what the previous publication left,
/// then writes the record.
///
/// # Errors
///
/// A store failure.
pub async fn write(
    store: &dyn BlobStore,
    dir: &BlobPath,
    site: &Site,
    mut record: Publication,
) -> Result<Publication, Error> {
    let started = Instant::now();
    let out = folder(dir, record.format)?;
    // mdBook reads pages and their images from `src/`.
    let asset_prefix = match record.format {
        PublishFormat::Site => "",
        PublishFormat::Mdbook => "src/",
    };
    let mut written = BTreeSet::new();
    for (rel, text) in &site.files {
        let path = out.join(rel).map_err(store_error)?;
        let _ = store
            .put(&path, Bytes::from(text.clone()), Precondition::None)
            .await
            .map_err(store_error)?;
        let _ = written.insert(path);
    }
    for asset in &site.assets {
        let Ok(from) = dir.join(asset) else { continue };
        let blob = match store.get(&from).await {
            Ok(blob) if blob.meta.size <= MAX_FILE_BYTES * 4 => blob,
            // A missing or oversized asset is a broken image, not a
            // failed publication.
            Ok(_) | Err(StoreError::NotFound(_) | StoreError::InvalidPath { .. }) => continue,
            Err(e) => return Err(store_error(e)),
        };
        let path = out
            .join(&format!("{asset_prefix}{asset}"))
            .map_err(store_error)?;
        let _ = store
            .put(&path, blob.bytes, Precondition::None)
            .await
            .map_err(store_error)?;
        let _ = written.insert(path);
    }
    let previous: Vec<_> = match store.list(Some(&out)).try_collect().await {
        Ok(entries) => entries,
        Err(StoreError::NotFound(_)) => Vec::new(),
        Err(e) => return Err(store_error(e)),
    };
    for entry in previous {
        if written.contains(&entry.path) {
            continue;
        }
        match store.delete(&entry.path, Precondition::None).await {
            Ok(()) | Err(StoreError::NotFound(_)) => {}
            Err(e) => return Err(store_error(e)),
        }
    }
    record.files = written.len();
    record.folder = out.as_str().to_owned();
    let json = serde_json::to_vec_pretty(&record)
        .map_err(|e| Error::new(ErrorKind::Internal, "publication record").with_source(e))?;
    let _ = store
        .put(
            &record_path(dir, record.format)?,
            Bytes::from(json),
            Precondition::None,
        )
        .await
        .map_err(store_error)?;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        histogram.vellum.books.store_ms = ms,
        books.op = "publish.write",
        books.files = record.files,
        "book store io"
    );
    Ok(record)
}

/// A file of a book's public site, for the public route: `None` unless
/// the book's site is published for everyone and the file exists.
///
/// # Errors
///
/// A store failure.
pub async fn public_file(
    store: &dyn BlobStore,
    dir: &BlobPath,
    rel: &str,
) -> Result<Option<Bytes>, Error> {
    let Some(record) = current(store, dir, PublishFormat::Site).await? else {
        return Ok(None);
    };
    if !record.public {
        return Ok(None);
    }
    let rel = if rel.is_empty() || rel.ends_with('/') {
        format!("{rel}index.html")
    } else {
        rel.to_owned()
    };
    // BlobPath refuses `..`, empty and absolute segments.
    let Ok(path) = folder(dir, PublishFormat::Site)?.join(&rel) else {
        return Ok(None);
    };
    match store.get(&path).await {
        Ok(blob) => Ok(Some(blob.bytes)),
        Err(StoreError::NotFound(_) | StoreError::InvalidPath { .. }) => Ok(None),
        Err(e) => Err(store_error(e)),
    }
}
