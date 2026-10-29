// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Applying changes made outside Vellum, as the store's watcher reports
//! them (D12). An edit in place becomes a version by the external editor;
//! anything that changes the tree (a create, rename, move, delete or copy)
//! rescans the spaces it touched, where identity follows the file's ID.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use futures::TryStreamExt;
use vellum_kernel::{Actor, ObjectId, SpaceId};
use vellum_store::BlobPath;
use vellum_store::watch::Change;

use super::Drive;
use crate::error::DriveResult;
use crate::frontmatter;
use crate::layout;
use crate::model::{ActivityKind, NodeKind, ScanReport, Space};

impl Drive {
    /// Applies a batch of watcher changes and returns what the rescans
    /// changed in the index.
    ///
    /// # Errors
    /// Store or index failures.
    #[tracing::instrument(name = "drive.apply_changes", skip_all, fields(changes = changes.len()), err)]
    pub async fn apply_changes(&self, changes: &[Change]) -> DriveResult<ScanReport> {
        self.observe("apply_changes", async {
            let spaces = self.index.spaces().await?;
            let mut rescan: BTreeSet<SpaceId> = BTreeSet::new();
            let mut moving: HashSet<ObjectId> = HashSet::new();
            for change in changes {
                // Sibling order is read from its file when listing, so a
                // changed order file needs no rescan (D169). Uploaded files
                // are not in the index at all (D211).
                let path = changed_path(change);
                if path.file_name() == layout::ORDER_FILE
                    || vellum_store::layout::in_attachments(path)
                {
                    continue;
                }
                match change {
                    Change::Modified { id, path, .. } => {
                        if !self.external_edit(*id, path).await? {
                            rescan.extend(space_of(&spaces, path));
                        }
                    }
                    Change::Renamed { id, from, to, .. } => {
                        let _ = moving.insert(*id);
                        rescan.extend(space_of(&spaces, from));
                        rescan.extend(space_of(&spaces, to));
                    }
                    Change::Created { path, .. }
                    | Change::Removed { path, .. }
                    | Change::Untracked { path, .. } => rescan.extend(space_of(&spaces, path)),
                    Change::Duplicated { copy, .. } => rescan.extend(space_of(&spaces, copy)),
                }
            }
            let mut report = ScanReport::default();
            for space in spaces.iter().filter(|s| rescan.contains(&s.id)) {
                self.rescan(space, &moving, &mut report).await?;
            }
            Ok(report)
        })
        .await
    }

    /// Records an in-place edit of a page as a version by the external
    /// editor. Returns false when the change is not a plain page edit and
    /// needs a rescan instead.
    async fn external_edit(&self, id: ObjectId, path: &BlobPath) -> DriveResult<bool> {
        let Some(mut node) = self.index.node(id).await? else {
            return Ok(false);
        };
        if !node.is_live() || node.kind != NodeKind::Page || &node.path != path {
            return Ok(false);
        }
        let (text, blob) = self.read_text(path).await?;
        // Our own saves come back through the watcher too.
        if let Some(latest) = self.index.versions(id).await?.first() {
            let (stored, _) = layout::version_paths(id, latest.id)?;
            if let Ok(prev) = self.store.get(&stored).await
                && prev.bytes == blob.bytes
            {
                return Ok(true);
            }
        }
        let actor = Actor::External;
        node.size = blob.meta.size;
        node.modified = self.clock.now();
        let version = self.write_version(&actor, &node, &blob.bytes).await?;
        self.index.put_node(&node).await?;
        self.reindex_links(&node, &text).await?;
        self.record(
            &actor,
            &node,
            ActivityKind::Edited {
                version: version.id,
            },
        )
        .await?;
        if let Some(title) = frontmatter::get_md(&text, "title") {
            let title = title.trim();
            if !title.is_empty() && title != node.title {
                let _ = self.retitle(&actor, node, title).await?;
            }
        }
        Ok(true)
    }

    /// Rescans one space against the files, treating IDs held by other
    /// spaces as taken unless they are moving here.
    async fn rescan(
        &self,
        space: &Space,
        moving: &HashSet<ObjectId>,
        report: &mut ScanReport,
    ) -> DriveResult<()> {
        let mut blobs: BTreeMap<BlobPath, u64> = BTreeMap::new();
        for root in [space.root.clone(), layout::trash_root()?] {
            let entries: Vec<_> = self.store.list(Some(&root)).try_collect().await?;
            blobs.extend(entries.into_iter().map(|e| (e.path, e.size)));
        }
        let mut seen = HashSet::new();
        for other in self.index.spaces().await? {
            if other.id == space.id {
                continue;
            }
            for node in self.index.nodes(other.id).await? {
                if node.is_live() && !moving.contains(&node.id) {
                    let _ = seen.insert(node.id);
                }
            }
        }
        let manifests = self.read_manifests(&blobs, report).await;
        self.scan_space(space, &blobs, &manifests, &mut seen, report)
            .await?;
        let ids: Vec<ObjectId> = self
            .index
            .nodes(space.id)
            .await?
            .into_iter()
            .map(|n| n.id)
            .collect();
        self.access.tree_changed(space.id, &ids).await
    }
}

/// The path a change leaves behind.
const fn changed_path(change: &Change) -> &BlobPath {
    match change {
        Change::Modified { path, .. }
        | Change::Created { path, .. }
        | Change::Removed { path, .. }
        | Change::Untracked { path, .. } => path,
        Change::Renamed { to, .. } => to,
        Change::Duplicated { copy, .. } => copy,
    }
}

/// The space whose directory holds `path`.
fn space_of(spaces: &[Space], path: &BlobPath) -> Option<SpaceId> {
    spaces
        .iter()
        .find(|s| path.starts_with(&s.root))
        .map(|s| s.id)
}
