// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The trash (R-DR-4): 30 days per drive, cascading with an
//! `explicitly_trashed` flag so restore is exact, and purge for managers.
//!
//! Trashing moves a node's files to `.vellum/trash/<id>/item/` and writes
//! a manifest listing it and every node that went with it. A node trashed
//! on its own earlier already sits in its own entry, so restoring its
//! folder later leaves it in the trash, exactly as it was.

use vellum_kernel::{Actor, ObjectId, SpaceId};
use vellum_store::{BlobPath, Precondition, StoreError};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{TrashManifest, TrashMember, to_yaml};
use crate::layout;
use crate::model::{ActivityKind, Node, TrashState};
use crate::ports::Permission;

const DAY_MS: i64 = 86_400_000;

impl Drive {
    /// Moves a node, and for a folder everything still in it, to the trash.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(
        name = "drive.trash",
        skip_all,
        fields(node = %id),
        err
    )]
    pub async fn trash(
        &self,
        actor: &Actor,
        id: ObjectId,
    ) -> DriveResult<Node> {
        self.observe("trash", async {
            let node = self.live_node(id).await?;
            let members = self
                .authorize_subtree(actor, &node, Permission::Edit)
                .await?;
            let entry = layout::trash_dir(node.id)?;
            let item = layout::trash_item(node.id, node.path.file_name())?;
            let now = self.clock.now();
            let manifest = TrashManifest {
                space: node.space,
                trashed_at: now,
                trashed_by: *actor,
                members: members
                    .iter()
                    .map(|m| TrashMember {
                        id: m.id,
                        kind: m.kind,
                        title: m.title.clone(),
                        original_path: m.path.clone(),
                        original_parent: m.parent,
                        created_by: m.created_by,
                    })
                    .collect(),
            };
            // Manifest first: a crash after it but before the move leaves
            // an entry the scan can reconcile, never a lost file.
            let manifest_path = entry.join(layout::TRASH_MANIFEST)?;
            let _ = self.put_file(&manifest_path, to_yaml(&manifest)?).await?;
            if let Err(err) =
                self.move_blobs(node.kind, &node.path, &item).await
            {
                let _ =
                    self.store.delete(&manifest_path, Precondition::None).await;
                return Err(err);
            }
            let mut trashed = node.clone();
            for member in members {
                let mut after = member.clone();
                after.path = layout::rebase_member(
                    node.kind,
                    &member.path,
                    &node.path,
                    &item,
                )?;
                after.trash = Some(TrashState {
                    trashed_at: now,
                    trashed_by: *actor,
                    explicitly_trashed: member.id == node.id,
                    entry: node.id,
                    original_path: member.path.clone(),
                    original_parent: member.parent,
                });
                self.index.put_node(&after).await?;
                if member.id == node.id {
                    trashed = after;
                }
            }
            self.access.tree_changed(node.space, &[node.id]).await?;
            self.record(actor, &trashed, ActivityKind::Trashed).await?;
            Ok(trashed)
        })
        .await
    }

    /// Brings an explicitly trashed node back, with exactly the nodes that
    /// went with it. If its folder is gone it lands at the space root; if
    /// its name is taken it gets a ` (2)` suffix.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` for a node that went with
    /// a folder (restore the folder instead), `QuotaExceeded` never (trash
    /// already counts against the quota).
    #[tracing::instrument(
        name = "drive.restore",
        skip_all,
        fields(node = %id),
        err
    )]
    pub async fn restore(
        &self,
        actor: &Actor,
        id: ObjectId,
    ) -> DriveResult<Node> {
        self.observe("restore", async {
            let node = self.any_node(id).await?;
            let state = self.entry_state(&node)?;
            self.authorize(actor, node.space, node.id, Permission::Edit)
                .await?;
            let space = self.space(node.space).await?;
            let parent = match state.original_parent {
                Some(parent) => match self.index.node(parent).await? {
                    Some(folder)
                        if folder.is_live() && folder.space == space.id =>
                    {
                        Some(parent)
                    }
                    _ => None,
                },
                None => None,
            };
            let dir = self.dir_of(&space, parent).await?;
            let path =
                self.free_path(&dir, node.kind, &node.title, None).await?;
            self.move_blobs(node.kind, &node.path, &path).await?;
            let mut restored = node.clone();
            for member in self.entry_members(&node).await? {
                let mut after = member.clone();
                after.path = layout::rebase_member(
                    node.kind,
                    &member.path,
                    &node.path,
                    &path,
                )?;
                after.trash = None;
                if member.id == node.id {
                    after.parent = parent;
                    after.modified = self.clock.now();
                } else if let Some(trash) = &member.trash {
                    after.parent = trash.original_parent;
                }
                self.index.put_node(&after).await?;
                if member.id == node.id {
                    restored = after;
                }
            }
            self.remove_entry_files(node.id).await?;
            self.access.tree_changed(space.id, &[node.id]).await?;
            self.record(actor, &restored, ActivityKind::Restored)
                .await?;
            Ok(restored)
        })
        .await
    }

    /// Deletes an explicitly trashed node and everything that went with
    /// it, versions included. Managers only.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` for a live node or one
    /// that went with a folder.
    #[tracing::instrument(
        name = "drive.purge",
        skip_all,
        fields(node = %id),
        err
    )]
    pub async fn purge(&self, actor: &Actor, id: ObjectId) -> DriveResult<()> {
        self.observe("purge", async {
            let node = self.any_node(id).await?;
            let _ = self.entry_state(&node)?;
            let space = self.space(node.space).await?;
            self.authorize(
                actor,
                space.id,
                space.node_id(),
                Permission::Manage,
            )
            .await?;
            self.purge_entry(actor, &node).await
        })
        .await
    }

    /// Purges every entry in a space's trash. Managers only. Returns how
    /// many entries went.
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied`.
    #[tracing::instrument(
        name = "drive.empty_trash",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn empty_trash(
        &self,
        actor: &Actor,
        space: SpaceId,
    ) -> DriveResult<usize> {
        self.observe("empty_trash", async {
            let space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                space.node_id(),
                Permission::Manage,
            )
            .await?;
            let entries = self.entries(space.id).await?;
            for entry in &entries {
                self.purge_entry(actor, entry).await?;
            }
            Ok(entries.len())
        })
        .await
    }

    /// Purges trash entries older than each space's `trash_days`. Run by
    /// the scheduler as Vellum itself. Returns how many entries went.
    ///
    /// # Errors
    /// Index or store failures.
    #[tracing::instrument(name = "drive.purge_expired", skip_all, err)]
    pub async fn purge_expired(&self) -> DriveResult<usize> {
        self.observe("purge_expired", async {
            let now = self.clock.now().unix_ms();
            let mut purged = 0;
            for space in self.index.spaces().await? {
                let keep_ms =
                    i64::from(space.settings.trash_days).saturating_mul(DAY_MS);
                for entry in self.entries(space.id).await? {
                    let expired = entry.trash.as_ref().is_some_and(|t| {
                        now.saturating_sub(t.trashed_at.unix_ms()) >= keep_ms
                    });
                    if expired {
                        self.purge_entry(&Actor::System, &entry).await?;
                        purged += 1;
                    }
                }
            }
            Ok(purged)
        })
        .await
    }

    /// A space's trash: the explicitly trashed nodes the caller may
    /// restore, newest first.
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied` without view rights on the space.
    #[tracing::instrument(
        name = "drive.trash_list",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn trash_list(
        &self,
        actor: &Actor,
        space: SpaceId,
    ) -> DriveResult<Vec<Node>> {
        self.observe("trash_list", async {
            let space = self.space(space).await?;
            self.authorize(actor, space.id, space.node_id(), Permission::View)
                .await?;
            let mut out = Vec::new();
            for entry in self.entries(space.id).await? {
                if self.may(actor, &entry, Permission::Edit).await? {
                    out.push(entry);
                }
            }
            out.sort_by(|a, b| {
                let at = |n: &Node| n.trash.as_ref().map(|t| t.trashed_at);
                at(b).cmp(&at(a))
            });
            Ok(out)
        })
        .await
    }

    /// The explicitly trashed nodes of a space.
    async fn entries(&self, space: SpaceId) -> DriveResult<Vec<Node>> {
        Ok(self
            .index
            .nodes(space)
            .await?
            .into_iter()
            .filter(|n| n.trash.as_ref().is_some_and(|t| t.explicitly_trashed))
            .collect())
    }

    /// The nodes held by `node`'s trash entry.
    async fn entry_members(&self, node: &Node) -> DriveResult<Vec<Node>> {
        Ok(self
            .index
            .nodes(node.space)
            .await?
            .into_iter()
            .filter(|n| n.trash.as_ref().is_some_and(|t| t.entry == node.id))
            .collect())
    }

    /// The trash state of an explicitly trashed node.
    fn entry_state(&self, node: &Node) -> DriveResult<TrashState> {
        let _ = self;
        match &node.trash {
            Some(state) if state.explicitly_trashed => Ok(state.clone()),
            Some(state) => Err(DriveError::Invalid(format!(
                "this item went to the trash with {}; restore or purge that \
                    instead",
                state.entry
            ))),
            None => {
                Err(DriveError::Invalid("this item is not in the trash".into()))
            }
        }
    }

    async fn purge_entry(&self, actor: &Actor, node: &Node) -> DriveResult<()> {
        let members = self.entry_members(node).await?;
        for member in &members {
            for version in self.index.versions(member.id).await? {
                self.delete_version(member.id, version.id).await?;
            }
        }
        let dir = layout::trash_dir(node.id)?;
        for (path, _) in self.blobs_under(&dir).await? {
            self.delete_quietly(&path).await?;
        }
        for member in &members {
            self.index.remove_node(member.id).await?;
        }
        let ids: Vec<ObjectId> = members.iter().map(|m| m.id).collect();
        self.access.tree_changed(node.space, &ids).await?;
        self.record(actor, node, ActivityKind::Purged).await
    }

    async fn remove_entry_files(&self, id: ObjectId) -> DriveResult<()> {
        for (path, _) in self.blobs_under(&layout::trash_dir(id)?).await? {
            self.delete_quietly(&path).await?;
        }
        Ok(())
    }

    async fn delete_quietly(&self, path: &BlobPath) -> DriveResult<()> {
        match self.store.delete(path, Precondition::None).await {
            Ok(()) | Err(StoreError::NotFound(_)) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}
