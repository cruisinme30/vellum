// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Detecting changes made outside Vellum.
//!
//! A change feed (inotify / `FSEvents` for a local folder; S3 events or a
//! Drive change feed later) reports which paths were touched. The
//! [`Reconciler`] re-reads those paths and turns them into [`Change`]s by
//! identity: the front-matter ULID, never the path. A file that disappears
//! from one path and appears at another with the same id is a rename, even
//! when the two halves arrive in different batches, so removals wait one
//! grace period before they are reported.

mod feed;
mod identity;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::{Duration, Instant},
};

use opentelemetry::{KeyValue, metrics::Counter};
use vellum_kernel::ObjectId;

pub use feed::{LocalFeed, run};
pub use identity::identity;

use crate::{
    error::{StoreError, StoreResult},
    layout::Kind,
    path::BlobPath,
    store::{BlobStore, Etag},
};

/// How long a removal waits for the matching half of a rename.
pub const RENAME_GRACE: Duration = Duration::from_secs(2);

/// A change made outside Vellum, by identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A new object appeared.
    Created {
        /// Its identity.
        id: ObjectId,
        /// Where it is.
        path: BlobPath,
        /// Version found.
        etag: Etag,
    },
    /// An object's content changed in place.
    Modified {
        /// Its identity.
        id: ObjectId,
        /// Where it is.
        path: BlobPath,
        /// Version found.
        etag: Etag,
    },
    /// An object moved; its content may also have changed.
    Renamed {
        /// Its identity.
        id: ObjectId,
        /// Previous path.
        from: BlobPath,
        /// New path.
        to: BlobPath,
        /// Version found.
        etag: Etag,
    },
    /// An object is gone.
    Removed {
        /// Its identity.
        id: ObjectId,
        /// Its last path.
        path: BlobPath,
    },
    /// A file was copied outside Vellum and carries an id already in use.
    /// The copy must get a fresh id; the original keeps it.
    Duplicated {
        /// The shared identity.
        id: ObjectId,
        /// The file that owns the id.
        original: BlobPath,
        /// The copy that must be re-identified.
        copy: BlobPath,
    },
    /// Content without an id (an asset, or a document with no front
    /// matter yet) appeared, changed or vanished.
    Untracked {
        /// Where.
        path: BlobPath,
        /// Whether it exists now.
        exists: bool,
    },
}

impl Change {
    const fn label(&self) -> &'static str {
        match self {
            Self::Created { .. } => "created",
            Self::Modified { .. } => "modified",
            Self::Renamed { .. } => "renamed",
            Self::Removed { .. } => "removed",
            Self::Duplicated { .. } => "duplicated",
            Self::Untracked { .. } => "untracked",
        }
    }
}

#[derive(Debug)]
struct Known {
    path: BlobPath,
    etag: Etag,
}

/// Turns touched paths into identity-level [`Change`]s.
#[derive(Debug)]
pub struct Reconciler<S> {
    store: S,
    by_id: HashMap<ObjectId, Known>,
    by_path: BTreeMap<BlobPath, ObjectId>,
    removals: Vec<(ObjectId, BlobPath, Instant)>,
    changes: Counter<u64>,
}

impl<S: BlobStore> Reconciler<S> {
    /// Starts from what the index already knows, as `(id, path, etag)`.
    pub fn new(
        store: S,
        known: impl IntoIterator<Item = (ObjectId, BlobPath, Etag)>,
    ) -> Self {
        let meter = opentelemetry::global::meter("vellum");
        let changes = meter
            .u64_counter("vellum_store_watch_changes_total")
            .with_description("External changes detected, by kind")
            .build();
        let mut this = Self {
            store,
            by_id: HashMap::new(),
            by_path: BTreeMap::new(),
            removals: Vec::new(),
            changes,
        };
        for (id, path, etag) in known {
            this.record(id, path, etag);
        }
        this
    }

    /// Notes a write Vellum made itself, so the feed's echo of it is not
    /// reported as an external change.
    pub fn record(&mut self, id: ObjectId, path: BlobPath, etag: Etag) {
        let known = Known {
            path: path.clone(),
            etag,
        };
        if let Some(old) = self.by_id.insert(id, known)
            && old.path != path
        {
            let _ = self.by_path.remove(&old.path);
        }
        let _ = self.by_path.insert(path, id);
        self.removals.retain(|(r, ..)| *r != id);
    }

    /// Compares the whole store with what is known: run at startup to
    /// catch edits made while Vellum was not running. Missing objects are
    /// reported as removed at once.
    ///
    /// # Errors
    /// Returns listing or read errors.
    #[tracing::instrument(skip_all, fields(changes), err)]
    pub async fn scan(&mut self) -> StoreResult<Vec<Change>> {
        use futures::TryStreamExt;
        let listed: BTreeSet<BlobPath> = self
            .store
            .list(None)
            .map_ok(|e| e.path)
            .try_collect()
            .await?;
        let mut changes = self
            .reconcile(listed.iter().cloned(), Instant::now())
            .await?;
        let gone: Vec<ObjectId> = self
            .by_id
            .iter()
            .filter(|(_, k)| !listed.contains(&k.path))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            changes.extend(self.remove(id));
        }
        self.removals.clear();
        self.count(&changes);
        Ok(changes)
    }

    /// Re-reads `paths` and reports what changed, plus removals whose
    /// grace period ended by `now`.
    ///
    /// # Errors
    /// Returns read errors other than `NotFound`.
    #[tracing::instrument(skip_all, fields(changes), err)]
    pub async fn reconcile(
        &mut self,
        paths: impl IntoIterator<Item = BlobPath>,
        now: Instant,
    ) -> StoreResult<Vec<Change>> {
        let paths = self.expand(paths).await?;
        let mut changes = Vec::new();
        for path in paths {
            changes.extend(self.touch(path, now).await?);
        }
        let due: Vec<(ObjectId, BlobPath)> = self
            .removals
            .iter()
            .filter(|(.., at)| *at <= now)
            .map(|(id, p, _)| (*id, p.clone()))
            .collect();
        self.removals.retain(|(.., at)| *at > now);
        for (id, path) in due {
            if self.by_id.get(&id).is_some_and(|k| k.path == path) {
                changes.extend(self.remove(id));
            }
        }
        let _ = tracing::Span::current().record("changes", changes.len());
        self.count(&changes);
        Ok(changes)
    }

    /// Feeds report a moved or deleted directory as the directory alone,
    /// so each touched path also stands for every file known beneath it
    /// and every file now beneath it. Directories themselves are dropped.
    async fn expand(
        &self,
        paths: impl IntoIterator<Item = BlobPath>,
    ) -> StoreResult<BTreeSet<BlobPath>> {
        use futures::TryStreamExt;
        let mut out = BTreeSet::new();
        for path in paths {
            let known = self
                .by_path
                .range(path.clone()..)
                .map(|(p, _)| p)
                .take_while(|p| p.starts_with(&path));
            let known: Vec<BlobPath> = known.cloned().collect();
            let present: Vec<BlobPath> = self
                .store
                .list(Some(&path))
                .map_ok(|e| e.path)
                .try_collect()
                .await?;
            let is_dir = known.iter().chain(&present).any(|p| *p != path);
            out.extend(known.into_iter().chain(present));
            if !is_dir {
                let _ = out.insert(path);
            }
        }
        out.retain(|p| Kind::of(p).is_content());
        Ok(out)
    }

    /// The earliest moment a pending removal falls due, if any.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.removals.iter().map(|(.., at)| *at).min()
    }

    async fn touch(
        &mut self,
        path: BlobPath,
        now: Instant,
    ) -> StoreResult<Option<Change>> {
        let blob = match self.store.get(&path).await {
            Ok(blob) => blob,
            Err(StoreError::NotFound(_)) => {
                if let Some(id) = self.by_path.get(&path) {
                    self.removals.push((*id, path, now + RENAME_GRACE));
                    return Ok(None);
                }
                return Ok(Some(Change::Untracked {
                    path,
                    exists: false,
                }));
            }
            Err(e) => return Err(e),
        };
        let etag = blob.meta.etag;
        let Some(id) = identity(&path, &blob.bytes) else {
            return Ok(Some(Change::Untracked { path, exists: true }));
        };
        let change = match self.by_id.get(&id) {
            None => Change::Created {
                id,
                path: path.clone(),
                etag: etag.clone(),
            },
            Some(k) if k.path == path && k.etag == etag => return Ok(None),
            Some(k) if k.path == path => Change::Modified {
                id,
                path: path.clone(),
                etag: etag.clone(),
            },
            Some(k) => {
                let original = k.path.clone();
                if self.still_has(&original, id).await? {
                    return Ok(Some(Change::Duplicated {
                        id,
                        original,
                        copy: path,
                    }));
                }
                Change::Renamed {
                    id,
                    from: original,
                    to: path.clone(),
                    etag: etag.clone(),
                }
            }
        };
        self.record(id, path, etag);
        Ok(Some(change))
    }

    async fn still_has(
        &self,
        path: &BlobPath,
        id: ObjectId,
    ) -> StoreResult<bool> {
        match self.store.get(path).await {
            Ok(blob) => Ok(identity(path, &blob.bytes) == Some(id)),
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn remove(&mut self, id: ObjectId) -> Option<Change> {
        let known = self.by_id.remove(&id)?;
        let _ = self.by_path.remove(&known.path);
        Some(Change::Removed {
            id,
            path: known.path,
        })
    }

    fn count(&self, changes: &[Change]) {
        for c in changes {
            self.changes.add(1, &[KeyValue::new("change", c.label())]);
        }
    }
}
