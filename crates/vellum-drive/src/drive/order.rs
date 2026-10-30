// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sibling order (D169) and the whole tree of a space in one answer (D170).
//!
//! The order of a container's items lives in the files, so a rebuilt index
//! loses nothing: a `.vellum-order.yaml` in the container's directory (the
//! space root, a folder, or the directory beside a page) lists child IDs
//! in order. Items it doesn't list, such as files added outside Vellum,
//! come after the listed ones, folders first and then by title. IDs that
//! are no longer children are ignored.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vellum_kernel::{Actor, ObjectId, SpaceId};
use vellum_store::{BlobPath, StoreError};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{from_yaml, to_yaml};
use crate::layout;
use crate::model::{Node, NodeKind, Space};
use crate::ports::Permission;

/// The most items a tree returns.
pub const MAX_TREE_ITEMS: usize = 10_000;

/// `.vellum-order.yaml`: the order of a container's items.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct OrderFile {
    /// Child IDs, first to last.
    order: Vec<ObjectId>,
}

/// A space's tree, or the part of it under one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    /// The items the caller may view, depth first, siblings in order.
    pub items: Vec<Node>,
    /// True when the limit cut the tree short.
    pub truncated: bool,
}

impl Drive {
    /// The directory that holds the items under `parent` in `space`.
    async fn container_dir(
        &self,
        space: &Space,
        parent: Option<&Node>,
    ) -> DriveResult<Option<BlobPath>> {
        match parent {
            None => Ok(Some(space.root.clone())),
            Some(node) => self.children_dir(node.kind, &node.path).await,
        }
    }

    /// The stored order of the items in `dir`; empty when there is none or
    /// the file can't be read.
    async fn stored_order(&self, dir: &BlobPath) -> DriveResult<Vec<ObjectId>> {
        let path = dir.join(layout::ORDER_FILE)?;
        match self.store.get(&path).await {
            Ok(blob) => Ok(from_yaml::<OrderFile>(&path, &blob.bytes)
                .map_or_else(
                    |err| {
                        tracing::warn!(%path, error = %err, "ignoring an \
                        unreadable order file");
                        Vec::new()
                    },
                    |f| f.order,
                )),
            Err(StoreError::NotFound(_)) => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    /// Sorts the items of the container at `dir` into their stored order.
    pub(super) async fn sort_siblings(
        &self,
        dir: Option<&BlobPath>,
        items: &mut [Node],
    ) -> DriveResult<()> {
        let order = match dir {
            Some(dir) => self.stored_order(dir).await?,
            None => Vec::new(),
        };
        sort_by_order(items, &order);
        Ok(())
    }

    /// Puts `id` at `index` among the live items under `parent` (the
    /// others keep their order), and stores the result.
    pub(super) async fn place(
        &self,
        space: &Space,
        parent: Option<ObjectId>,
        id: ObjectId,
        index: usize,
    ) -> DriveResult<()> {
        let holder = match parent {
            Some(p) => Some(self.live_node(p).await?),
            None => None,
        };
        let Some(dir) = self.container_dir(space, holder.as_ref()).await?
        else {
            return Ok(());
        };
        let mut siblings = self.index.children(space.id, parent).await?;
        self.sort_siblings(Some(&dir), &mut siblings).await?;
        let mut order: Vec<ObjectId> =
            siblings.iter().map(|n| n.id).filter(|n| *n != id).collect();
        order.insert(index.min(order.len()), id);
        let _ = self
            .put_file(
                &dir.join(layout::ORDER_FILE)?,
                to_yaml(&OrderFile { order })?,
            )
            .await?;
        Ok(())
    }

    /// Moves an item to `index` among its siblings without changing its
    /// parent.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied` without edit rights on the parent.
    #[tracing::instrument(
        name = "drive.reorder",
        skip_all,
        fields(node = %id),
        err
    )]
    pub async fn reorder(
        &self,
        actor: &Actor,
        id: ObjectId,
        index: usize,
    ) -> DriveResult<Node> {
        self.observe("reorder", async {
            let node = self.live_node(id).await?;
            let space = self.space(node.space).await?;
            self.authorize(
                actor,
                space.id,
                node.parent.unwrap_or_else(|| space.node_id()),
                Permission::Edit,
            )
            .await?;
            self.place(&space, node.parent, node.id, index).await?;
            Ok(node)
        })
        .await
    }

    /// Every live item under `root` (`None`: the whole space) that the
    /// caller may view, depth first with siblings in order, down to
    /// `depth` levels and at most `limit` items. A hidden item hides
    /// everything beneath it.
    ///
    /// # Errors
    /// `SpaceNotFound`, `NotFound`, `Invalid` for a root that holds
    /// nothing, `PermissionDenied`.
    #[tracing::instrument(
        name = "drive.tree",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn tree(
        &self,
        actor: &Actor,
        space: SpaceId,
        root: Option<ObjectId>,
        depth: Option<usize>,
        limit: usize,
    ) -> DriveResult<Tree> {
        self.observe("tree", async {
            let space = self.space(space).await?;
            let root_node = match root {
                Some(id) => {
                    let node = self.live_node(id).await?;
                    if node.space != space.id {
                        return Err(DriveError::NotFound(id));
                    }
                    Some(node)
                }
                None => None,
            };
            let _ = self.dir_of(&space, root).await?;
            self.authorize(
                actor,
                space.id,
                root.unwrap_or_else(|| space.node_id()),
                Permission::View,
            )
            .await?;
            let mut children: HashMap<Option<ObjectId>, Vec<Node>> =
                HashMap::new();
            for node in self.index.nodes(space.id).await? {
                if node.is_live() {
                    children.entry(node.parent).or_default().push(node);
                }
            }
            let limit = limit.clamp(1, MAX_TREE_ITEMS);
            let mut tree = Tree {
                items: Vec::new(),
                truncated: false,
            };
            // Each entry: a parent, its items still to visit (reversed, so
            // pop gives the next), and how deep they are.
            let first = self
                .ordered(&space, root_node.as_ref(), &mut children)
                .await?;
            let mut stack: Vec<(Vec<Node>, usize)> = vec![(first, 1)];
            while let Some((items, level)) = stack.last_mut() {
                let level = *level;
                let Some(node) = items.pop() else {
                    let _ = stack.pop();
                    continue;
                };
                if !self.may(actor, &node, Permission::View).await? {
                    continue;
                }
                if tree.items.len() >= limit {
                    tree.truncated = true;
                    break;
                }
                let deeper = depth.is_none_or(|d| level < d);
                let below = if deeper && children.contains_key(&Some(node.id)) {
                    Some(
                        self.ordered(&space, Some(&node), &mut children)
                            .await?,
                    )
                } else {
                    None
                };
                tree.items.push(node);
                if let Some(below) = below {
                    stack.push((below, level + 1));
                }
            }
            Ok(tree)
        })
        .await
    }

    /// Takes `parent`'s items out of `children`, in order, reversed.
    async fn ordered(
        &self,
        space: &Space,
        parent: Option<&Node>,
        children: &mut HashMap<Option<ObjectId>, Vec<Node>>,
    ) -> DriveResult<Vec<Node>> {
        let mut items =
            children.remove(&parent.map(|n| n.id)).unwrap_or_default();
        let dir = self.container_dir(space, parent).await?;
        self.sort_siblings(dir.as_ref(), &mut items).await?;
        items.reverse();
        Ok(items)
    }
}

/// Sorts items by their place in `order`; the rest follow, folders first,
/// then by title.
fn sort_by_order(items: &mut [Node], order: &[ObjectId]) {
    let rank: HashMap<ObjectId, usize> =
        order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    items.sort_by(|a, b| {
        let (ra, rb) = (rank.get(&a.id), rank.get(&b.id));
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => (a.kind != NodeKind::Folder)
                .cmp(&(b.kind != NodeKind::Folder))
                .then_with(|| {
                    a.title.to_lowercase().cmp(&b.title.to_lowercase())
                })
                .then_with(|| a.id.cmp(&b.id)),
        }
    });
}
