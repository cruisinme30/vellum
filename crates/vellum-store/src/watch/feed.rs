// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Change feeds and the debounce loop that drives a [`Reconciler`].

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

use super::{Change, Reconciler};
use crate::{
    error::{StoreError, StoreResult},
    layout::Kind,
    path::{BlobPath, TEMP_PREFIX},
    store::BlobStore,
};

/// Quiet period that ends a batch: editors save in bursts (write temp,
/// rename, touch), and one reconcile per burst is enough.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Longest a batch gathers, so a folder that never goes quiet (a sync
/// client, a build) still gets reconciled.
const MAX_BATCH_WAIT: Duration = Duration::from_secs(2);

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
        let root: PathBuf =
            root.canonicalize().map_err(|e| StoreError::Io {
                path: root.display().to_string(),
                source: e,
            })?;
        let base = root.clone();
        let mut watcher = notify::recommended_watcher(
            move |res: notify::Result<notify::Event>| match res {
                // Reads are not changes (closing a written file is).
                // Reporting them would also loop: the reconciler lists
                // folders, which the OS reports as reads.
                Ok(event) if is_read(event.kind) => {}
                Ok(event) => {
                    for path in
                        event.paths.iter().filter_map(|p| relative(&base, p))
                    {
                        if tx.blocking_send(path).is_err() {
                            return; // The reconciler has stopped.
                        }
                    }
                }
                Err(e) => tracing::warn!(error = %e, "filesystem watch error"),
            },
        )
        .map_err(|e| {
            StoreError::Backend(format!("cannot start watcher: {e}"))
        })?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| {
                StoreError::Backend(format!(
                    "cannot watch {}: {e}",
                    root.display()
                ))
            })?;
        tracing::info!(root = %root.display(), "watching for external changes");
        Ok(Self { _watcher: watcher })
    }
}

fn is_read(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Access(
            access
        ) if access != AccessKind::Close(AccessMode::Write)
    )
}

fn relative(root: &Path, file: &Path) -> Option<BlobPath> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> =
        rel.components().map(|c| c.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    if joined
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .starts_with(TEMP_PREFIX)
    {
        return None;
    }
    let path = BlobPath::new(joined).ok()?;
    // Vellum's own state (the index, versions) changes on every save.
    (Kind::of(&path) != Kind::System).then_some(path)
}

/// Drives `reconciler` from a feed until either channel closes.
///
/// It gathers touched paths until the feed is quiet for a moment, reconciles
/// them, and sends the resulting changes in order. Pending removals are flushed
/// when their grace period ends even if the feed stays quiet.
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
            Some(due) => {
                tokio::time::timeout_at(due.into(), feed.recv()).await.ok()
            }
            None => Some(feed.recv().await),
        };
        match first {
            Some(None) => return Ok(()), // The feed closed.
            Some(Some(path)) => batch.push(path),
            None => {} // A removal fell due.
        }
        let cutoff = tokio::time::Instant::now() + MAX_BATCH_WAIT;
        while let Ok(Some(path)) = tokio::time::timeout(
            DEBOUNCE.min(cutoff - tokio::time::Instant::now()),
            feed.recv(),
        )
        .await
        {
            batch.push(path);
            if tokio::time::Instant::now() >= cutoff {
                break;
            }
        }
        for change in reconciler.reconcile(batch, Instant::now()).await? {
            if out.send(change).await.is_err() {
                return Ok(()); // Nobody is listening any more.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, AccessMode, CreateKind};

    use super::*;

    #[test]
    fn reads_are_not_changes_but_finished_writes_are() {
        assert!(is_read(EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(is_read(EventKind::Access(AccessKind::Close(
            AccessMode::Read
        ))));
        assert!(!is_read(EventKind::Access(AccessKind::Close(
            AccessMode::Write
        ))));
        assert!(!is_read(EventKind::Create(CreateKind::File)));
    }

    #[test]
    fn vellum_state_and_temp_files_are_not_reported() {
        let root = Path::new("/data");
        let rel = |p: &str| {
            relative(root, &root.join(p)).map(|b| b.as_str().to_owned())
        };
        assert_eq!(rel("drives/Plan.md").as_deref(), Some("drives/Plan.md"));
        assert_eq!(rel(".vellum/index.sqlite-wal"), None);
        assert_eq!(rel(".vellum/versions/01M63X7PTPE44TNDZ490X8R47P"), None);
        assert_eq!(rel(&format!("drives/{TEMP_PREFIX}x.md")), None);
    }
}
