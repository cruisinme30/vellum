// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The ports the drive needs from the rest of the system.
//!
//! The files in the [`vellum_store::BlobStore`] are the truth. The
//! [`DriveIndex`] is a rebuildable projection of them (nodes, links,
//! versions) plus the state files do not carry (stars, recents, activity).
//! [`Access`] is the authorization service. Adapters: [`crate::memory`] for
//! tests and the zero-config binary; SQL and `vellum-authz` adapters are
//! wired in by the server.

use std::fmt;

use async_trait::async_trait;
use vellum_kernel::{ObjectId, SpaceId, Timestamp, UserId, VersionId};
use vellum_store::BlobPath;

use crate::error::DriveResult;
use crate::model::{Activity, Node, Space, VersionRecord};

/// The drive's index: nodes, per-user state, activity, links and versions.
#[async_trait]
pub trait DriveIndex: Send + Sync + fmt::Debug {
    /// Inserts or replaces a space.
    async fn put_space(&self, space: &Space) -> DriveResult<()>;
    /// Looks up a space.
    async fn space(&self, id: SpaceId) -> DriveResult<Option<Space>>;
    /// Every space.
    async fn spaces(&self) -> DriveResult<Vec<Space>>;

    /// Inserts or replaces a node.
    async fn put_node(&self, node: &Node) -> DriveResult<()>;
    /// Looks up a node, live or trashed.
    async fn node(&self, id: ObjectId) -> DriveResult<Option<Node>>;
    /// The node at `path` in `space`, live or trashed.
    async fn node_at(&self, space: SpaceId, path: &BlobPath) -> DriveResult<Option<Node>>;
    /// Live nodes directly in `parent` (`None`: the space root).
    async fn children(&self, space: SpaceId, parent: Option<ObjectId>) -> DriveResult<Vec<Node>>;
    /// Every node in a space, live and trashed.
    async fn nodes(&self, space: SpaceId) -> DriveResult<Vec<Node>>;
    /// Live nodes in `space` with this exact title.
    async fn by_title(&self, space: SpaceId, title: &str) -> DriveResult<Vec<Node>>;
    /// Removes a node and everything keyed by it (links from it, user
    /// state, versions); activity is kept as history.
    async fn remove_node(&self, id: ObjectId) -> DriveResult<()>;

    /// Stars (`Some`) or unstars (`None`) a node for a user.
    async fn set_starred(
        &self,
        user: UserId,
        node: ObjectId,
        at: Option<Timestamp>,
    ) -> DriveResult<()>;
    /// A user's starred nodes, most recently starred first.
    async fn starred(&self, user: UserId) -> DriveResult<Vec<ObjectId>>;
    /// Records that a user opened or edited a node.
    async fn touch(&self, user: UserId, node: ObjectId, at: Timestamp) -> DriveResult<()>;
    /// A user's most recently opened or edited nodes, newest first.
    async fn recent(&self, user: UserId, limit: usize) -> DriveResult<Vec<(ObjectId, Timestamp)>>;

    /// Appends an activity entry.
    async fn append_activity(&self, entry: &Activity) -> DriveResult<()>;
    /// Activity on `node` or anything beneath it, newest first.
    async fn activity(&self, node: ObjectId, limit: usize) -> DriveResult<Vec<Activity>>;

    /// Replaces the set of nodes `source` links to.
    async fn set_links(&self, source: ObjectId, targets: &[ObjectId]) -> DriveResult<()>;
    /// Nodes that link to `target`.
    async fn backlinks(&self, target: ObjectId) -> DriveResult<Vec<ObjectId>>;

    /// Records a version.
    async fn put_version(&self, version: &VersionRecord) -> DriveResult<()>;
    /// A page's versions, newest first.
    async fn versions(&self, doc: ObjectId) -> DriveResult<Vec<VersionRecord>>;
    /// Forgets a version.
    async fn remove_version(&self, id: VersionId) -> DriveResult<()>;

    /// Bytes used by a space's nodes (live and trashed) and versions.
    async fn usage(&self, space: SpaceId) -> DriveResult<u64>;
}

/// What a caller wants to do with a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Open, list and download.
    View,
    /// Change content, create, rename, move within reach, trash.
    Edit,
    /// Purge, change space settings, move out of a shared drive.
    Manage,
}

/// The authorization service, as the drive sees it.
///
/// Inheritance lives behind this port: a node's access is its own grants
/// plus its ancestors'. A moved node keeps the grants made on it and
/// loses the ones it inherited, because inherited access is recomputed from
/// the new ancestors once [`Access::tree_changed`] is called (R-DR-2).
#[async_trait]
pub trait Access: Send + Sync + fmt::Debug {
    /// Whether `user` holds `permission` on `node` (the space's own ID for
    /// the space root). Deny by default.
    async fn check(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        permission: Permission,
    ) -> DriveResult<bool>;
    /// Spaces where `user` holds any access.
    async fn spaces(&self, user: UserId) -> DriveResult<Vec<SpaceId>>;
    /// Nodes in `space` that `user` may view.
    async fn accessible(&self, user: UserId, space: SpaceId) -> DriveResult<Vec<ObjectId>>;
    /// Called after nodes were created, moved, trashed, restored or purged,
    /// so materialised access can be recomputed for their subtrees.
    async fn tree_changed(&self, space: SpaceId, nodes: &[ObjectId]) -> DriveResult<()>;
    /// Gives the creator of a node ownership of it.
    async fn grant_owner(&self, user: UserId, space: SpaceId, node: ObjectId) -> DriveResult<()>;
}

/// The source of the current time, replaceable in tests.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}
