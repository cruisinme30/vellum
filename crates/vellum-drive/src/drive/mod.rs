// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive service.
//!
//! Every operation takes the [`Actor`] doing it, checks permission through
//! the [`Access`] port (deny by default), changes the files in the
//! [`BlobStore`] first and the [`DriveIndex`] second, and records an
//! activity entry. Files are the truth: if a crash lands between the two,
//! [`Drive::scan`] brings the index back in line.

mod books;
mod content;
mod create;
mod organize;
mod scan;
mod trash;
mod views;
mod watch;

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use futures::TryStreamExt;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};
use vellum_kernel::{Actor, EventId, ObjectId, SpaceId, UserId};
use vellum_store::{BlobPath, BlobStore, StoreError};

use crate::error::{DriveError, DriveResult};
use crate::layout;
use crate::model::{Activity, ActivityKind, Node, NodeKind, Space};
use crate::ports::{Access, Clock, DriveIndex, Permission, SystemClock};

pub use content::{SaveOutcome, Saved};
pub use views::Filter;

/// Counter of drive operations, labelled `op` and `outcome`.
pub const OPS_TOTAL: &str = "vellum_drive_ops_total";
/// Histogram of drive operation latency in seconds, labelled `op`.
pub const OP_SECONDS: &str = "vellum_drive_op_duration_seconds";

/// The drive service: spaces and pages (brief §6).
#[derive(Debug, Clone)]
pub struct Drive {
    store: Arc<dyn BlobStore>,
    index: Arc<dyn DriveIndex>,
    access: Arc<dyn Access>,
    clock: Arc<dyn Clock>,
    metrics: Metrics,
}

#[derive(Debug, Clone)]
struct Metrics {
    ops: Counter<u64>,
    seconds: Histogram<f64>,
}

impl Metrics {
    fn new() -> Self {
        let meter = opentelemetry::global::meter("vellum");
        Self {
            ops: meter
                .u64_counter(OPS_TOTAL)
                .with_description("Drive operations")
                .build(),
            seconds: meter
                .f64_histogram(OP_SECONDS)
                .with_unit("s")
                .with_description("Drive operation latency")
                .build(),
        }
    }
}

impl Drive {
    /// Builds the service over its ports, using the system clock.
    #[must_use]
    pub fn new(
        store: Arc<dyn BlobStore>,
        index: Arc<dyn DriveIndex>,
        access: Arc<dyn Access>,
    ) -> Self {
        Self::with_clock(store, index, access, Arc::new(SystemClock))
    }

    /// Builds the service with a custom clock (tests, simulations).
    #[must_use]
    pub fn with_clock(
        store: Arc<dyn BlobStore>,
        index: Arc<dyn DriveIndex>,
        access: Arc<dyn Access>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            index,
            access,
            clock,
            metrics: Metrics::new(),
        }
    }

    /// Runs `fut` as operation `op`, recording its outcome and latency.
    async fn observe<T>(
        &self,
        op: &'static str,
        fut: impl Future<Output = DriveResult<T>>,
    ) -> DriveResult<T> {
        let started = Instant::now();
        let result = fut.await;
        let outcome = match &result {
            Ok(_) => "ok",
            Err(err) => err.code(),
        };
        self.metrics.ops.add(
            1,
            &[KeyValue::new("op", op), KeyValue::new("outcome", outcome)],
        );
        self.metrics
            .seconds
            .record(started.elapsed().as_secs_f64(), &[KeyValue::new("op", op)]);
        if let Err(err) = &result {
            tracing::debug!(op, error = %err, "drive operation failed");
        }
        result
    }

    /// Fails unless `actor` holds `permission` on `node` in `space`.
    ///
    /// People are checked through the access port. A model acting for a
    /// person gets at most that person's view and edit rights: organising
    /// and purging stay with people. The file watcher and Vellum itself
    /// act with full rights.
    async fn authorize(
        &self,
        actor: &Actor,
        space: SpaceId,
        node: ObjectId,
        permission: Permission,
    ) -> DriveResult<()> {
        let user = match actor {
            Actor::User { user } => *user,
            Actor::Model { on_behalf_of } if permission != Permission::Manage => *on_behalf_of,
            Actor::Model { .. } => return Err(DriveError::PermissionDenied),
            _ => return Ok(()),
        };
        if self.access.check(user, space, node, permission).await? {
            Ok(())
        } else {
            Err(DriveError::PermissionDenied)
        }
    }

    /// Like [`Self::authorize`] but answers instead of failing.
    async fn may(&self, actor: &Actor, node: &Node, permission: Permission) -> DriveResult<bool> {
        match self.authorize(actor, node.space, node.id, permission).await {
            Ok(()) => Ok(true),
            Err(DriveError::PermissionDenied) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// The space, or `SpaceNotFound`.
    async fn space(&self, id: SpaceId) -> DriveResult<Space> {
        self.index
            .space(id)
            .await?
            .ok_or(DriveError::SpaceNotFound(id))
    }

    /// The node, live or trashed, or `NotFound`.
    async fn any_node(&self, id: ObjectId) -> DriveResult<Node> {
        self.index.node(id).await?.ok_or(DriveError::NotFound(id))
    }

    /// The node if it is live, or `NotFound`.
    async fn live_node(&self, id: ObjectId) -> DriveResult<Node> {
        let node = self.any_node(id).await?;
        if node.is_live() {
            Ok(node)
        } else {
            Err(DriveError::NotFound(id))
        }
    }

    /// The directory a new child of `parent` goes in, checking that the
    /// parent is a live folder of `space`.
    async fn dir_of(&self, space: &Space, parent: Option<ObjectId>) -> DriveResult<BlobPath> {
        let Some(parent) = parent else {
            return Ok(space.root.clone());
        };
        let folder = self.live_node(parent).await?;
        if folder.space != space.id || !folder.kind.is_container() {
            return Err(DriveError::Invalid(format!(
                "{parent} is not a folder in this space"
            )));
        }
        Ok(folder.path)
    }

    /// The node's folders, innermost first, then the space itself.
    async fn ancestors(&self, node: &Node) -> DriveResult<Vec<ObjectId>> {
        let mut out = Vec::new();
        let mut parent = node.parent;
        while let Some(id) = parent {
            if out.contains(&id) {
                break;
            }
            out.push(id);
            parent = self.index.node(id).await?.and_then(|n| n.parent);
        }
        out.push(node.space.into());
        Ok(out)
    }

    /// Appends an activity entry for `node`.
    async fn record(&self, actor: &Actor, node: &Node, kind: ActivityKind) -> DriveResult<()> {
        let entry = Activity {
            id: EventId::generate(),
            at: self.clock.now(),
            actor: *actor,
            space: node.space,
            node: node.id,
            ancestors: self.ancestors(node).await?,
            kind,
        };
        self.index.append_activity(&entry).await
    }

    /// Fails if adding `bytes` would take `space` over its quota.
    async fn ensure_quota(&self, space: &Space, bytes: u64) -> DriveResult<()> {
        let Some(quota) = space.settings.quota_bytes else {
            return Ok(());
        };
        let used = self.index.usage(space.id).await?;
        if bytes > 0 && used.saturating_add(bytes) > quota {
            return Err(DriveError::QuotaExceeded {
                used,
                quota,
                needed: bytes,
            });
        }
        Ok(())
    }

    /// True if anything exists at `path`, as a file or a directory.
    async fn occupied(&self, path: &BlobPath) -> DriveResult<bool> {
        match self.store.head(path).await {
            Ok(_) => return Ok(true),
            Err(StoreError::NotFound(_)) => {}
            Err(err) => return Err(err.into()),
        }
        let mut entries = self.store.list(Some(path));
        Ok(entries.try_next().await?.is_some())
    }

    /// The first free path for a `kind` node titled `title` in `dir`,
    /// suffixing ` (2)`, ` (3)`… on collision. `keep` is a path that counts
    /// as free (the node's own, when renaming).
    async fn free_path(
        &self,
        dir: &BlobPath,
        kind: NodeKind,
        title: &str,
        keep: Option<&BlobPath>,
    ) -> DriveResult<BlobPath> {
        for counter in 1..=10_000 {
            let path = dir.join(&layout::file_name(kind, title, counter))?;
            if keep == Some(&path) || !self.occupied(&path).await? {
                return Ok(path);
            }
        }
        Err(DriveError::AlreadyExists(title.to_owned()))
    }

    /// Every blob at or beneath `path`.
    async fn blobs_under(&self, path: &BlobPath) -> DriveResult<Vec<(BlobPath, u64)>> {
        let entries: Vec<_> = self.store.list(Some(path)).try_collect().await?;
        let mut out: Vec<(BlobPath, u64)> = entries.into_iter().map(|e| (e.path, e.size)).collect();
        out.sort();
        Ok(out)
    }

    /// Moves every blob of a node (one file, or a whole directory) from
    /// `from` to `to`, undoing the moves already made if one fails.
    async fn move_blobs(&self, kind: NodeKind, from: &BlobPath, to: &BlobPath) -> DriveResult<()> {
        if from == to {
            return Ok(());
        }
        if !kind.is_dir() {
            let _ = self.store.rename(from, to).await?;
            return Ok(());
        }
        let blobs = self.blobs_under(from).await?;
        let mut done: Vec<(BlobPath, BlobPath)> = Vec::with_capacity(blobs.len());
        for (path, _) in blobs {
            let target = layout::rebase(&path, from, to)?;
            match self.store.rename(&path, &target).await {
                Ok(_) => done.push((path, target)),
                Err(err) => {
                    for (back, moved) in done.iter().rev() {
                        if let Err(undo) = self.store.rename(moved, back).await {
                            tracing::error!(path = %moved, error = %undo, "could not undo a partial move");
                        }
                    }
                    return Err(err.into());
                }
            }
        }
        Ok(())
    }

    /// Reads a blob as UTF-8 text.
    async fn read_text(&self, path: &BlobPath) -> DriveResult<(String, vellum_store::Blob)> {
        let blob = self.store.get(path).await?;
        let text = String::from_utf8(blob.bytes.to_vec()).map_err(|_| DriveError::Malformed {
            path: path.to_string(),
            reason: "not UTF-8".to_owned(),
        })?;
        Ok((text, blob))
    }

    /// The person behind an actor, if any.
    const fn user_of(actor: &Actor) -> Option<UserId> {
        match actor {
            Actor::User { user } => Some(*user),
            Actor::Model { on_behalf_of } => Some(*on_behalf_of),
            _ => None,
        }
    }

    /// Writes a small drive-owned file unconditionally.
    async fn put_file(&self, path: &BlobPath, text: String) -> DriveResult<u64> {
        let meta = self
            .store
            .put(path, Bytes::from(text), vellum_store::Precondition::None)
            .await?;
        Ok(meta.size)
    }
}
