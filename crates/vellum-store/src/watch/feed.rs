// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Change feeds and the debounce loop that drives a [`Reconciler`].

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

use super::{Change, Reconciler};
use crate::{
    error::{StoreError, StoreResult},
    path::{BlobPath, TEMP_PREFIX},
    store::BlobStore,
};

/// Quiet period that ends a batch: editors save in bursts (write temp,
/// rename, touch), and one reconcile per burst is enough.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Reports paths touched under a local folder, using inotify, `FSEvents`
/// or `ReadDirectoryChangesW`. Dropping it stops the watch.
#[derive(Debug)]
pub struct LocalFeed {
    _watcher: RecommendedWatcher,
}

impl LocalFeed {
    /// Watches `root` recursively and sends each touched path to `tx`.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if the OS watch cannot start.
    pub fn start(root: &Path, tx: mpsc::Sender<BlobPath>) -> StoreResult<Self> {
        let root: PathBuf = root.canonicalize().map_err(|e| StoreError::Io {
            path: root.display().to_string(),
            source: e,
        })?;
        let base = root.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    for path in event.paths.iter().filter_map(|p| relative(&base, p)) {
                        if tx.blocking_send(path).is_err() {
                            return; // The reconciler has stopped.
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "filesystem watch error"),
            })
            .map_err(|e| StoreError::Backend(format!("cannot start watcher: {e}")))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| StoreError::Backend(format!("cannot watch {}: {e}", root.display())))?;
        tracing::info!(root = %root.display(), "watching for external changes");
        Ok(Self { _watcher: watcher })
    }
}

fn relative(root: &Path, file: &Path) -> Option<BlobPath> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    if joined
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .starts_with(TEMP_PREFIX)
    {
        return None;
    }
    BlobPath::new(joined).ok()
}

/// Drives `reconciler` from a feed until either channel closes.
///
/// It gathers touched paths until the feed is quiet for a moment, reconciles them,
/// and sends the resulting changes in order. Pending removals are
/// flushed when their grace period ends even if the feed stays quiet.
///
/// # Errors
/// Returns the first store error from reconciling.
pub async fn run<S: BlobStore>(
    mut reconciler: Reconciler<S>,
    mut feed: mpsc::Receiver<BlobPath>,
    out: mpsc::Sender<Change>,
) -> StoreResult<()> {
    loop {
        let mut batch = Vec::new();
        let first = match reconciler.next_deadline() {
            Some(due) => tokio::time::timeout_at(due.into(), feed.recv()).await.ok(),
            None => Some(feed.recv().await),
        };
        match first {
            Some(None) => return Ok(()), // The feed closed.
            Some(Some(path)) => batch.push(path),
            None => {} // A removal fell due.
        }
        while let Ok(Some(path)) = tokio::time::timeout(DEBOUNCE, feed.recv()).await {
            batch.push(path);
        }
        for change in reconciler.reconcile(batch, Instant::now()).await? {
            if out.send(change).await.is_err() {
                return Ok(()); // Nobody is listening any more.
            }
        }
    }
}
