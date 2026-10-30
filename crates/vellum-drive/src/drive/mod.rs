// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive service.
//!
//! Every operation takes the [`Actor`] doing it, checks permission through
//! the [`Access`] port (deny by default), changes the files in the
//! [`BlobStore`] first and the [`DriveIndex`] second, and records an
//! activity entry. Files are the truth: if a crash lands between the two,
//! [`Drive::scan`] brings the index back in line.

mod admin;
mod attach;
mod books;
mod content;
mod create;
mod import;
mod order;
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
use crate::ports::{
    Access, Clock, ContentCheck, DriveIndex, Permission, SystemClock,
};

pub use admin::AdminSpace;
pub use attach::{Attachment, MAX_ATTACHMENTS};
pub use content::{SaveOutcome, Saved};
pub use create::MY_DRIVE_NAME;
pub use import::{Imported, MAX_IMPORT_FILES};
pub use order::{MAX_TREE_ITEMS, Tree};
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
    check: Option<Arc<dyn ContentCheck>>,
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
            check: None,
        }
    }

    /// Checks every page text before it is written (R-SE-6).
    #[must_use]
    pub fn with_content_check(mut self, check: Arc<dyn ContentCheck>) -> Self {
        self.check = Some(check);
        self
    }

    /// Runs the content check, if there is one, on page text `bytes`.
    async fn check_content(
        &self,
        actor: &Actor,
        space: SpaceId,
        path: &BlobPath,
        bytes: &[u8],
    ) -> DriveResult<()> {
        match &self.check {
            Some(check) => {
                check
                    .check(actor, space, path, &String::from_utf8_lossy(bytes))
                    .await
            }
            None => Ok(()),
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
        self.metrics.seconds.record(
            started.elapsed().as_secs_f64(),
            &[KeyValue::new("op", op)],
        );
        if let Err(err) = &result {
            tracing::debug!(op, error = %err, "drive operation failed");
        }
        result
    }

    /// Whether the index answers, for readiness probes.
    ///
    /// # Errors
    /// The index failure.
    pub async fn ready(&self) -> DriveResult<()> {
        self.index.spaces().await.map(|_| ())
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
            Actor::Model { on_behalf_of }
                if permission != Permission::Manage =>
            {
                *on_behalf_of
            }
            Actor::Model { .. } => return Err(DriveError::PermissionDenied),
            _ => return Ok(()),
        };
        if self.access.check(user, space, node, permission).await? {
            Ok(())
        } else {
            Err(DriveError::PermissionDenied)
        }
    }

    /// Fails unless `actor` holds `permission` on `node` and on every live
    /// node beneath it, which it returns. Moving or trashing a folder
    /// carries everything in it, so a part the actor may not change (a
    /// restricted page, say) blocks the whole operation. The answer stays
    /// `PermissionDenied` so it does not reveal what is hidden.
    async fn authorize_subtree(
        &self,
        actor: &Actor,
        node: &Node,
        permission: Permission,
    ) -> DriveResult<Vec<Node>> {
        let members = self.subtree(node).await?;
        for member in &members {
            self.authorize(actor, member.space, member.id, permission)
                .await?;
        }
        Ok(members)
    }

    /// Like [`Self::authorize`] but answers instead of failing.
    async fn may(
        &self,
        actor: &Actor,
        node: &Node,
        permission: Permission,
    ) -> DriveResult<bool> {
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
    /// parent is a live folder or page of `space`.
    async fn dir_of(
        &self,
        space: &Space,
        parent: Option<ObjectId>,
    ) -> DriveResult<BlobPath> {
        let Some(parent) = parent else {
            return Ok(space.root.clone());
        };
        let holder = self.live_node(parent).await?;
        if holder.space != space.id {
            return Err(DriveError::Invalid(format!(
                "{parent} is not a folder or page in this space"
            )));
        }
        self.children_dir(holder.kind, &holder.path)
            .await?
            .ok_or_else(|| {
                DriveError::Invalid(format!(
                    "{parent} is not a folder or page in this space"
                ))
            })
    }

    /// The directory holding what is nested under a node: a folder's own
    /// directory, or the directory beside a page (D168). A page whose
    /// directory is a folder in its own right (it has a
    /// `.vellum-folder.yaml`, made outside Vellum) holds nothing.
    pub(super) async fn children_dir(
        &self,
        kind: NodeKind,
        path: &BlobPath,
    ) -> DriveResult<Option<BlobPath>> {
        match kind {
            NodeKind::Folder => Ok(Some(path.clone())),
            NodeKind::Page => {
                let Some(dir) = layout::page_dir(path) else {
                    return Ok(None);
                };
                match self.store.head(&dir.join(layout::FOLDER_FILE)?).await {
                    Ok(_) => Ok(None),
                    Err(StoreError::NotFound(_)) => Ok(Some(dir)),
                    Err(err) => Err(err.into()),
                }
            }
            NodeKind::Book | NodeKind::Collection | NodeKind::Shortcut => {
                Ok(None)
            }
        }
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
    async fn record(
        &self,
        actor: &Actor,
        node: &Node,
        kind: ActivityKind,
    ) -> DriveResult<()> {
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
            if keep == Some(&path) {
                return Ok(path);
            }
            // A page and a folder of the same name would claim the same
            // directory (D168), so each needs the other's name free too.
            let twin = match kind {
                NodeKind::Page => layout::page_dir(&path),
                NodeKind::Folder => {
                    Some(dir.join(&format!("{}.md", path.file_name()))?)
                }
                NodeKind::Book | NodeKind::Collection | NodeKind::Shortcut => {
                    None
                }
            };
            let twin_taken = match &twin {
                Some(twin) => self.occupied(twin).await?,
                None => false,
            };
            if !twin_taken && !self.occupied(&path).await? {
                return Ok(path);
            }
        }
        Err(DriveError::AlreadyExists(title.to_owned()))
    }

    /// Every blob at or beneath `path`.
    async fn blobs_under(
        &self,
        path: &BlobPath,
    ) -> DriveResult<Vec<(BlobPath, u64)>> {
        let entries: Vec<_> = self.store.list(Some(path)).try_collect().await?;
        let mut out: Vec<(BlobPath, u64)> =
            entries.into_iter().map(|e| (e.path, e.size)).collect();
        out.sort();
        Ok(out)
    }

    /// Moves every blob of a node (one file, a whole directory, or a page
    /// with the pages nested under it) from `from` to `to`, undoing the
    /// moves already made if one fails.
    async fn move_blobs(
        &self,
        kind: NodeKind,
        from: &BlobPath,
        to: &BlobPath,
    ) -> DriveResult<()> {
        if from == to {
            return Ok(());
        }
        let mut moves: Vec<(BlobPath, BlobPath)> = Vec::new();
        if kind.is_dir() {
            for (path, _) in self.blobs_under(from).await? {
                let target = layout::rebase(&path, from, to)?;
                moves.push((path, target));
            }
        } else {
            moves.push((from.clone(), to.clone()));
            if let (Some(old), Some(new)) =
                (self.children_dir(kind, from).await?, layout::page_dir(to))
            {
                for (path, _) in self.blobs_under(&old).await? {
                    let target = layout::rebase(&path, &old, &new)?;
                    moves.push((path, target));
                }
            }
        }
        let mut done: Vec<(BlobPath, BlobPath)> =
            Vec::with_capacity(moves.len());
        for (path, target) in moves {
            match self.store.rename(&path, &target).await {
                Ok(_) => done.push((path, target)),
                Err(err) => {
                    for (back, moved) in done.iter().rev() {
                        if let Err(undo) = self.store.rename(moved, back).await
                        {
                            tracing::error!(
                                path = %moved,
                                error = %undo,
                                "could not undo a partial move"
                            );
                        }
                    }
                    return Err(err.into());
                }
            }
        }
        Ok(())
    }

    /// Reads a blob as UTF-8 text.
    async fn read_text(
        &self,
        path: &BlobPath,
    ) -> DriveResult<(String, vellum_store::Blob)> {
        let blob = self.store.get(path).await?;
        let text = String::from_utf8(blob.bytes.to_vec()).map_err(|_| {
            DriveError::Malformed {
                path: path.to_string(),
                reason: "not UTF-8".to_owned(),
            }
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
    async fn put_file(
        &self,
        path: &BlobPath,
        text: String,
    ) -> DriveResult<u64> {
        let meta = self
            .store
            .put(path, Bytes::from(text), vellum_store::Precondition::None)
            .await?;
        Ok(meta.size)
    }
}
