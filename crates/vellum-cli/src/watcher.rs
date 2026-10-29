// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps the index in step with edits made outside Vellum.
//!
//! People edit the data folder with any editor, Git or a sync client. The
//! store's watcher turns the paths the OS reports into identity-level
//! changes; this module hands them to the services that care:
//!
//! - the drive records in-place page edits as versions by an external
//!   editor, and rescans the spaces whose tree changed;
//! - the books service re-reads where books live and re-applies a book's
//!   section restrictions when its `book.yaml` changes.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;
use vellum_books_service::BooksService;
use vellum_drive::Drive;
use vellum_kernel::{Error, ErrorKind};
use vellum_store::layout::Kind;
use vellum_store::watch::{Change, LocalFeed, Reconciler, run};
use vellum_store::{BlobPath, BlobStore, LocalStore};

/// Paths waiting for the reconciler, and changes waiting to be applied.
const QUEUE: usize = 4096;

/// Most changes applied in one batch.
const BATCH: usize = 512;

/// Starts watching `data_dir` and applying what changes. A watch the OS
/// refuses (too many watches, an unsupported file system) is logged and
/// skipped: the server still runs, and a restart rescans.
///
/// # Errors
///
/// The data folder cannot be opened or listed.
pub async fn start(
    data_dir: &Path,
    files: &Arc<dyn BlobStore>,
    drive: &Arc<Drive>,
    books: &Arc<BooksService>,
) -> Result<(), Error> {
    let (paths_tx, paths_rx) = mpsc::channel(QUEUE);
    let feed = match LocalFeed::start(data_dir, paths_tx) {
        Ok(feed) => feed,
        Err(error) => {
            tracing::warn!(%error, "not watching for external edits; restart to pick them up");
            return Ok(());
        }
    };
    let store = LocalStore::open(data_dir).await.map_err(|e| {
        Error::new(
            ErrorKind::Unavailable,
            format!("cannot open the data folder: {e}"),
        )
    })?;
    // Learn every file's identity now, so the first edit is reported as
    // an edit. The drive's own rescan covers what is already on disk.
    let mut reconciler = Reconciler::new(store, std::iter::empty());
    let _ = reconciler.scan().await.map_err(|e| {
        Error::new(
            ErrorKind::Unavailable,
            format!("cannot list the data folder: {e}"),
        )
    })?;
    let (changes_tx, changes_rx) = mpsc::channel(QUEUE);
    drop(tokio::spawn(async move {
        let _feed = feed; // Dropping it stops the OS watch.
        if let Err(error) = run(reconciler, paths_rx, changes_tx).await {
            tracing::error!(%error, "file watcher stopped; restart to resume");
        }
    }));
    let services = Services {
        files: Arc::clone(files),
        drive: Arc::clone(drive),
        books: Arc::clone(books),
    };
    drop(tokio::spawn(apply_loop(changes_rx, services)));
    Ok(())
}

/// What the watcher updates.
struct Services {
    files: Arc<dyn BlobStore>,
    drive: Arc<Drive>,
    books: Arc<BooksService>,
}

/// Applies changes in batches until the watcher stops.
async fn apply_loop(mut changes: mpsc::Receiver<Change>, services: Services) {
    while let Some(first) = changes.recv().await {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match changes.try_recv() {
                Ok(change) => batch.push(change),
                Err(_) => break,
            }
        }
        apply(&batch, &services).await;
    }
}

/// Applies one batch. Failures are logged: one bad file must not stop
/// the watcher, and the next restart rescans everything.
async fn apply(batch: &[Change], services: &Services) {
    let Services {
        files,
        drive,
        books,
    } = services;
    match drive.apply_changes(batch).await {
        Ok(report) => tracing::debug!(?report, "applied external changes"),
        Err(error) => tracing::warn!(%error, "external changes not applied to the drive"),
    }
    let touched = book_manifests(batch);
    if touched.is_empty() {
        return;
    }
    // A book.yaml appeared, moved, changed or went: forget where books
    // are, then re-apply each touched book's section restrictions.
    books.invalidate().await;
    let located = match vellum_books_service::files::locate(files.as_ref()).await {
        Ok(located) => located,
        Err(error) => {
            tracing::warn!(%error, "books not re-read after a book.yaml change");
            return;
        }
    };
    for (id, dir) in located {
        if !touched.contains(&dir) {
            continue;
        }
        match books.sync_access(id).await {
            Ok(()) => tracing::debug!(book = %dir, "book access re-applied"),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(%error, book = %dir, "book access not re-applied"),
        }
    }
}

/// The book folders whose `book.yaml` or `SUMMARY.md` a batch touched.
fn book_manifests(batch: &[Change]) -> BTreeSet<BlobPath> {
    let mut out = BTreeSet::new();
    for change in batch {
        let paths: Vec<&BlobPath> = match change {
            Change::Created { path, .. }
            | Change::Modified { path, .. }
            | Change::Removed { path, .. }
            | Change::Untracked { path, .. } => vec![path],
            Change::Renamed { from, to, .. } => vec![from, to],
            Change::Duplicated { copy, .. } => vec![copy],
        };
        for path in paths {
            if let Kind::BookManifest { book } = Kind::of(path) {
                let _ = out.insert(book);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> BlobPath {
        BlobPath::new(p).unwrap()
    }

    #[test]
    fn only_book_manifests_touch_books() {
        let batch = vec![
            Change::Untracked {
                path: path("spaces/eng/guide.book/book.yaml"),
                exists: true,
            },
            Change::Untracked {
                path: path("spaces/eng/guide.book/intro.md"),
                exists: true,
            },
            Change::Untracked {
                path: path("spaces/eng/notes.md"),
                exists: false,
            },
            Change::Untracked {
                path: path("spaces/eng/manual.book/SUMMARY.md"),
                exists: false,
            },
        ];
        let books: Vec<String> = book_manifests(&batch)
            .into_iter()
            .map(|p| p.as_str().to_owned())
            .collect();
        assert_eq!(books, ["spaces/eng/guide.book", "spaces/eng/manual.book"]);
    }
}
