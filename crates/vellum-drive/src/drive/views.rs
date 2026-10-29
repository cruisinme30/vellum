// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive's navigation (R-DR-1): My Drive, Shared drives, Shared with
//! me, Starred, Recent, folder listings; plus activity, backlinks, stars
//! and usage. Every list is filtered by what the caller may view.

use std::collections::HashSet;

use vellum_kernel::{Actor, ObjectId, SpaceId, Timestamp, UserId};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::model::{Activity, Node, NodeKind, Space, SpaceKind, Usage};
use crate::ports::Permission;

impl Drive {
    /// A person's My Drive, if it has been created.
    ///
    /// # Errors
    /// Index failures.
    #[tracing::instrument(name = "drive.my_drive", skip_all, err)]
    pub async fn my_drive(&self, user: UserId) -> DriveResult<Option<Space>> {
        self.observe("my_drive", async {
            Ok(self
                .index
                .spaces()
                .await?
                .into_iter()
                .find(|s| s.kind == SpaceKind::UserDrive && s.owner == Some(user)))
        })
        .await
    }

    /// The shared drives a person can see, by name.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.shared_drives", skip_all, err)]
    pub async fn shared_drives(&self, user: UserId) -> DriveResult<Vec<Space>> {
        self.observe("shared_drives", async {
            let mut out = Vec::new();
            for id in self.access.spaces(user).await? {
                if let Some(space) = self.index.space(id).await?
                    && space.kind == SpaceKind::SharedDrive
                {
                    out.push(space);
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
        .await
    }

    /// Items in other people's My Drives shared with this person: the
    /// topmost accessible node of each shared subtree.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.shared_with_me", skip_all, err)]
    pub async fn shared_with_me(&self, user: UserId) -> DriveResult<Vec<Node>> {
        self.observe("shared_with_me", async {
            let mut out = Vec::new();
            for id in self.access.spaces(user).await? {
                let Some(space) = self.index.space(id).await? else {
                    continue;
                };
                if space.kind != SpaceKind::UserDrive || space.owner == Some(user) {
                    continue;
                }
                let visible: HashSet<ObjectId> = self
                    .access
                    .accessible(user, space.id)
                    .await?
                    .into_iter()
                    .collect();
                for node in self.index.nodes(space.id).await? {
                    let parent_visible = node.parent.is_some_and(|p| visible.contains(&p));
                    if node.is_live() && visible.contains(&node.id) && !parent_visible {
                        out.push(node);
                    }
                }
            }
            out.sort_by_key(|n| std::cmp::Reverse(n.modified));
            Ok(out)
        })
        .await
    }

    /// The live nodes directly in a folder or page (`None`: the space
    /// root) that the caller may view, in their stored order (D169), the
    /// rest folders first, then by title.
    ///
    /// # Errors
    /// `SpaceNotFound`, `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.list", skip_all, fields(space = %space), err)]
    pub async fn list(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<Vec<Node>> {
        self.observe("list", async {
            let space = self.space(space).await?;
            let dir = self.dir_of(&space, parent).await?;
            self.authorize(
                actor,
                space.id,
                parent.unwrap_or_else(|| space.node_id()),
                Permission::View,
            )
            .await?;
            let mut out = self
                .visible(actor, self.index.children(space.id, parent).await?)
                .await?;
            self.sort_siblings(Some(&dir), &mut out).await?;
            Ok(out)
        })
        .await
    }

    /// Looks up one node the caller may view, live or trashed.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.get", skip_all, fields(node = %id), err)]
    pub async fn get(&self, actor: &Actor, id: ObjectId) -> DriveResult<Node> {
        self.observe("get", async {
            let node = self.any_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            Ok(node)
        })
        .await
    }

    /// Follows a shortcut. The caller needs view rights on the target
    /// itself: a shortcut never grants access (R-DR-3).
    ///
    /// # Errors
    /// `NotFound` (also for a trashed target), `PermissionDenied`,
    /// `Invalid` if the node is not a shortcut.
    #[tracing::instrument(name = "drive.resolve_shortcut", skip_all, fields(node = %id), err)]
    pub async fn resolve_shortcut(&self, actor: &Actor, id: ObjectId) -> DriveResult<Node> {
        self.observe("resolve_shortcut", async {
            let shortcut = self.live_node(id).await?;
            self.authorize(actor, shortcut.space, shortcut.id, Permission::View)
                .await?;
            let target = shortcut
                .target
                .ok_or_else(|| DriveError::Invalid("not a shortcut".into()))?;
            let node = self.live_node(target).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            Ok(node)
        })
        .await
    }

    /// Stars or unstars a node for the caller.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` without a person.
    #[tracing::instrument(name = "drive.star", skip_all, fields(node = %id), err)]
    pub async fn star(&self, actor: &Actor, id: ObjectId, starred: bool) -> DriveResult<()> {
        self.observe("star", async {
            let user = Self::user_of(actor)
                .ok_or_else(|| DriveError::Invalid("only people star items".into()))?;
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            let at = starred.then(|| self.clock.now());
            self.index.set_starred(user, node.id, at).await
        })
        .await
    }

    /// The caller's starred items, most recently starred first.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.starred", skip_all, err)]
    pub async fn starred(&self, actor: &Actor) -> DriveResult<Vec<Node>> {
        self.observe("starred", async {
            let Some(user) = Self::user_of(actor) else {
                return Ok(Vec::new());
            };
            let mut nodes = Vec::new();
            for id in self.index.starred(user).await? {
                if let Some(node) = self.index.node(id).await?
                    && node.is_live()
                {
                    nodes.push(node);
                }
            }
            self.visible(actor, nodes).await
        })
        .await
    }

    /// The caller's recently opened or edited items, newest first.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.recent", skip_all, err)]
    pub async fn recent(&self, actor: &Actor, limit: usize) -> DriveResult<Vec<(Node, Timestamp)>> {
        self.observe("recent", async {
            let Some(user) = Self::user_of(actor) else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            // Over-fetch: some entries may be trashed or no longer visible.
            for (id, at) in self
                .index
                .recent(user, limit.saturating_mul(2).max(limit))
                .await?
            {
                if out.len() >= limit {
                    break;
                }
                let Some(node) = self.index.node(id).await? else {
                    continue;
                };
                if node.is_live()
                    && node.kind != NodeKind::Folder
                    && self.may(actor, &node, Permission::View).await?
                {
                    out.push((node, at));
                }
            }
            Ok(out)
        })
        .await
    }

    /// Activity on a node, or for a folder or space on anything beneath
    /// it, newest first (R-DR-7).
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.activity", skip_all, fields(node = %id), err)]
    pub async fn activity(
        &self,
        actor: &Actor,
        id: ObjectId,
        limit: usize,
    ) -> DriveResult<Vec<Activity>> {
        self.observe("activity", async {
            let (space, node_id) = if let Some(node) = self.index.node(id).await? {
                (node.space, node.id)
            } else {
                let space = self
                    .space(id.into())
                    .await
                    .map_err(|_| DriveError::NotFound(id))?;
                (space.id, space.node_id())
            };
            self.authorize(actor, space, node_id, Permission::View)
                .await?;
            let mut out = Vec::new();
            for entry in self
                .index
                .activity(node_id, limit.saturating_mul(2).max(limit))
                .await?
            {
                if out.len() >= limit {
                    break;
                }
                let visible = entry.node == node_id
                    || match self.index.node(entry.node).await? {
                        Some(node) => self.may(actor, &node, Permission::View).await?,
                        None => false,
                    };
                if visible {
                    out.push(entry);
                }
            }
            Ok(out)
        })
        .await
    }

    /// "Where is this used": live pages linking to a node, that the caller
    /// may view (R-DR-7).
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.backlinks", skip_all, fields(node = %id), err)]
    pub async fn backlinks(&self, actor: &Actor, id: ObjectId) -> DriveResult<Vec<Node>> {
        self.observe("backlinks", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            let mut sources = Vec::new();
            for source in self.index.backlinks(node.id).await? {
                if let Some(n) = self.index.node(source).await?
                    && n.is_live()
                {
                    sources.push(n);
                }
            }
            self.visible(actor, sources).await
        })
        .await
    }

    /// A space's storage use and quota (R-DR-10).
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.usage", skip_all, fields(space = %space), err)]
    pub async fn usage(&self, actor: &Actor, space: SpaceId) -> DriveResult<Usage> {
        self.observe("usage", async {
            let space = self.space(space).await?;
            self.authorize(actor, space.id, space.node_id(), Permission::View)
                .await?;
            Ok(Usage {
                used: self.index.usage(space.id).await?,
                quota: space.settings.quota_bytes,
            })
        })
        .await
    }

    /// Keeps the nodes the caller may view.
    async fn visible(&self, actor: &Actor, nodes: Vec<Node>) -> DriveResult<Vec<Node>> {
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            if self.may(actor, &node, Permission::View).await? {
                out.push(node);
            }
        }
        Ok(out)
    }
}

/// Drive search filters (R-DR-9). Empty fields do not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Case-insensitive text the title must contain.
    pub title: Option<String>,
    /// Who created the item.
    pub owner: Option<UserId>,
    /// Modified at or after.
    pub modified_after: Option<Timestamp>,
    /// Modified before.
    pub modified_before: Option<Timestamp>,
    /// Kinds to keep; empty keeps all.
    pub kinds: Vec<NodeKind>,
    /// A space or folder the item must be in, at any depth.
    pub location: Option<ObjectId>,
    /// Only items the caller starred.
    pub starred: bool,
}

impl Drive {
    /// Finds live items the caller may view that match `filter`, most
    /// recently modified first, up to `limit`.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.find", skip_all, err)]
    pub async fn find(
        &self,
        actor: &Actor,
        filter: &Filter,
        limit: usize,
    ) -> DriveResult<Vec<Node>> {
        self.observe("find", async {
            let spaces: Vec<SpaceId> = match Self::user_of(actor) {
                Some(user) => self.access.spaces(user).await?,
                None => self
                    .index
                    .spaces()
                    .await?
                    .into_iter()
                    .map(|s| s.id)
                    .collect(),
            };
            let starred: HashSet<ObjectId> = match (filter.starred, Self::user_of(actor)) {
                (true, Some(user)) => self.index.starred(user).await?.into_iter().collect(),
                _ => HashSet::new(),
            };
            let needle = filter.title.as_ref().map(|t| t.to_lowercase());
            let mut hits = Vec::new();
            for space in spaces {
                let nodes = self.index.nodes(space).await?;
                let location = match filter.location {
                    None => None,
                    Some(loc) if loc.as_u128() == space.as_u128() => None,
                    Some(loc) => match nodes.iter().find(|n| n.id == loc) {
                        Some(folder) => Some(folder.path.clone()),
                        None => continue,
                    },
                };
                for node in nodes {
                    let keep = node.is_live()
                        && filter.location.is_none_or(|loc| {
                            loc.as_u128() == space.as_u128()
                                || location
                                    .as_ref()
                                    .is_some_and(|p| node.path.starts_with(p) && node.id != loc)
                        })
                        && needle
                            .as_ref()
                            .is_none_or(|n| node.title.to_lowercase().contains(n))
                        && filter.owner.is_none_or(|o| node.created_by == Some(o))
                        && filter.modified_after.is_none_or(|t| node.modified >= t)
                        && filter.modified_before.is_none_or(|t| node.modified < t)
                        && (filter.kinds.is_empty() || filter.kinds.contains(&node.kind))
                        && (!filter.starred || starred.contains(&node.id));
                    if keep {
                        hits.push(node);
                    }
                }
            }
            hits.sort_by_key(|n| std::cmp::Reverse(n.modified));
            let mut out = Vec::new();
            for node in hits {
                if out.len() >= limit {
                    break;
                }
                if self.may(actor, &node, Permission::View).await? {
                    out.push(node);
                }
            }
            Ok(out)
        })
        .await
    }
}
