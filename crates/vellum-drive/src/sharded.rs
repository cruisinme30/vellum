// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A [`DriveIndex`] spread over shards (ADR 0040, D174).
//!
//! Each shard holds whole spaces, so a call naming a space goes to that
//! space's shard. A call naming only a node finds the node's shard first
//! (remembered once seen). Calls about a user across spaces (starred,
//! recent, backlinks, every space) ask every shard and merge.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use async_trait::async_trait;
use vellum_db::ShardRouter;
use vellum_kernel::{ObjectId, SpaceId, Timestamp, UserId, VersionId};
use vellum_store::BlobPath;

use crate::error::{DriveError, DriveResult};
use crate::model::{Activity, DriveTotals, Node, Space, VersionRecord};
use crate::ports::DriveIndex;
use crate::sql::SqlIndex;

/// The drive index over every shard of a [`ShardRouter`].
pub struct ShardedIndex {
    router: Arc<ShardRouter>,
    /// One adapter per shard, in the router's order.
    shards: Vec<SqlIndex>,
    /// Node to shard position, as far as this replica has seen.
    nodes: RwLock<HashMap<ObjectId, usize>>,
}

impl fmt::Debug for ShardedIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShardedIndex")
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

fn routing(err: impl fmt::Display) -> DriveError {
    DriveError::Index(err.to_string())
}

const fn object(space: SpaceId) -> ObjectId {
    ObjectId::from_uuid(space.as_uuid())
}

impl ShardedIndex {
    /// The drive tables on every shard of `router`.
    #[must_use]
    pub fn new(router: Arc<ShardRouter>) -> Self {
        let shards = router.indexes().cloned().map(SqlIndex::new).collect();
        Self {
            router,
            shards,
            nodes: RwLock::new(HashMap::new()),
        }
    }

    /// The adapter for each shard, home first.
    #[must_use]
    pub fn shards(&self) -> &[SqlIndex] {
        &self.shards
    }

    fn shard(&self, at: usize) -> DriveResult<&SqlIndex> {
        self.shards
            .get(at)
            .ok_or_else(|| DriveError::Index(format!("no shard #{at}")))
    }

    /// The shard storing `space`, placing it first when nobody has: a
    /// write may be the first anyone makes for a space.
    async fn for_write(&self, space: SpaceId) -> DriveResult<(usize, &SqlIndex)> {
        let at = self.router.place(object(space)).await.map_err(routing)?;
        Ok((at, self.shard(at)?))
    }

    async fn for_space(&self, space: SpaceId) -> DriveResult<&SqlIndex> {
        let at = self.router.position(object(space)).await.map_err(routing)?;
        self.shard(at)
    }

    fn learn(&self, node: ObjectId, at: usize) {
        if self.shards.len() == 1 {
            return;
        }
        let _ = self
            .nodes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(node, at);
    }

    fn forget(&self, node: ObjectId) {
        let _ = self
            .nodes
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&node);
    }

    /// The node and the position of the shard that holds it.
    async fn locate(&self, id: ObjectId) -> DriveResult<Option<(usize, Node)>> {
        if self.shards.len() == 1 {
            return Ok(self.shard(0)?.node(id).await?.map(|node| (0, node)));
        }
        let cached = self
            .nodes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .copied();
        if let Some(at) = cached
            && let Some(node) = self.shard(at)?.node(id).await?
        {
            return Ok(Some((at, node)));
        }
        for (at, shard) in self.shards.iter().enumerate() {
            if let Some(node) = shard.node(id).await? {
                self.learn(id, at);
                return Ok(Some((at, node)));
            }
        }
        Ok(None)
    }

    /// The shard other than `at` that holds `id`, if any: the node is
    /// moving between shards.
    async fn previous_shard(&self, id: ObjectId, at: usize) -> DriveResult<Option<usize>> {
        let cached = self
            .nodes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .copied();
        if cached == Some(at) || self.shards.len() == 1 {
            return Ok(None);
        }
        if cached.is_none() && self.shard(at)?.node(id).await?.is_some() {
            return Ok(None);
        }
        for (from, shard) in self.shards.iter().enumerate() {
            if from != at && shard.node(id).await?.is_some() {
                return Ok(Some(from));
            }
        }
        Ok(None)
    }

    /// The shard holding `id`, or home when no shard does (the call then
    /// behaves as on a single database).
    async fn for_node(&self, id: ObjectId) -> DriveResult<&SqlIndex> {
        if self.shards.len() == 1 {
            return self.shard(0);
        }
        let at = self.locate(id).await?.map_or(0, |(at, _)| at);
        self.shard(at)
    }
}

#[async_trait]
impl DriveIndex for ShardedIndex {
    async fn put_space(&self, space: &Space) -> DriveResult<()> {
        let at = self.router.place(object(space.id)).await.map_err(routing)?;
        self.shard(at)?.put_space(space).await
    }

    async fn space(&self, id: SpaceId) -> DriveResult<Option<Space>> {
        self.for_space(id).await?.space(id).await
    }

    async fn spaces(&self) -> DriveResult<Vec<Space>> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.spaces().await?);
        }
        Ok(all)
    }

    async fn put_node(&self, node: &Node) -> DriveResult<()> {
        let at = self.for_write(node.space).await?.0;
        let before = self.previous_shard(node.id, at).await?;
        self.shard(at)?.put_node(node).await?;
        if let Some(from) = before {
            // Moved into a space on another shard. The drive has already
            // copied the versions; stars, recents and outgoing links stay
            // behind with the old row and are dropped (ADR 0040, D174).
            self.shard(from)?.remove_node(node.id).await?;
        }
        self.learn(node.id, at);
        Ok(())
    }

    async fn node(&self, id: ObjectId) -> DriveResult<Option<Node>> {
        Ok(self.locate(id).await?.map(|(_, node)| node))
    }

    async fn node_at(&self, space: SpaceId, path: &BlobPath) -> DriveResult<Option<Node>> {
        self.for_space(space).await?.node_at(space, path).await
    }

    async fn children(&self, space: SpaceId, parent: Option<ObjectId>) -> DriveResult<Vec<Node>> {
        self.for_space(space).await?.children(space, parent).await
    }

    async fn nodes(&self, space: SpaceId) -> DriveResult<Vec<Node>> {
        self.for_space(space).await?.nodes(space).await
    }

    async fn by_title(&self, space: SpaceId, title: &str) -> DriveResult<Vec<Node>> {
        self.for_space(space).await?.by_title(space, title).await
    }

    async fn remove_node(&self, id: ObjectId) -> DriveResult<()> {
        match self.locate(id).await? {
            Some((at, _)) => self.shard(at)?.remove_node(id).await?,
            // Gone from the nodes table but maybe not from what hangs off
            // it: clear it everywhere, as removing is idempotent.
            None => {
                for shard in &self.shards {
                    shard.remove_node(id).await?;
                }
            }
        }
        self.forget(id);
        Ok(())
    }

    async fn set_starred(
        &self,
        user: UserId,
        node: ObjectId,
        at: Option<Timestamp>,
    ) -> DriveResult<()> {
        self.for_node(node).await?.set_starred(user, node, at).await
    }

    async fn starred(&self, user: UserId) -> DriveResult<Vec<ObjectId>> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.starred(user).await?);
        }
        Ok(all)
    }

    async fn touch(&self, user: UserId, node: ObjectId, at: Timestamp) -> DriveResult<()> {
        self.for_node(node).await?.touch(user, node, at).await
    }

    async fn recent(&self, user: UserId, limit: usize) -> DriveResult<Vec<(ObjectId, Timestamp)>> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.recent(user, limit).await?);
        }
        all.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
        all.truncate(limit);
        Ok(all)
    }

    async fn append_activity(&self, entry: &Activity) -> DriveResult<()> {
        self.for_write(entry.space)
            .await?
            .1
            .append_activity(entry)
            .await
    }

    async fn activity(&self, node: ObjectId, limit: usize) -> DriveResult<Vec<Activity>> {
        self.for_node(node).await?.activity(node, limit).await
    }

    async fn set_links(&self, source: ObjectId, targets: &[ObjectId]) -> DriveResult<()> {
        self.for_node(source)
            .await?
            .set_links(source, targets)
            .await
    }

    async fn backlinks(&self, target: ObjectId) -> DriveResult<Vec<ObjectId>> {
        // A link can cross spaces, so the source may be on any shard.
        let mut all = BTreeSet::new();
        for shard in &self.shards {
            all.extend(shard.backlinks(target).await?);
        }
        Ok(all.into_iter().collect())
    }

    async fn put_version(&self, version: &VersionRecord) -> DriveResult<()> {
        self.for_write(version.space)
            .await?
            .1
            .put_version(version)
            .await
    }

    async fn versions(&self, doc: ObjectId) -> DriveResult<Vec<VersionRecord>> {
        self.for_node(doc).await?.versions(doc).await
    }

    async fn remove_version(&self, id: VersionId) -> DriveResult<()> {
        for shard in &self.shards {
            shard.remove_version(id).await?;
        }
        Ok(())
    }

    async fn usage(&self, space: SpaceId) -> DriveResult<u64> {
        self.for_space(space).await?.usage(space).await
    }

    /// Each shard holds whole spaces, so the totals are the shards' sum.
    async fn totals(&self, now: Timestamp) -> DriveResult<DriveTotals> {
        let mut sum = DriveTotals::default();
        for shard in &self.shards {
            let t = shard.totals(now).await?;
            sum.spaces = sum.spaces.saturating_add(t.spaces);
            sum.folders = sum.folders.saturating_add(t.folders);
            sum.pages = sum.pages.saturating_add(t.pages);
            sum.books = sum.books.saturating_add(t.books);
            sum.collections = sum.collections.saturating_add(t.collections);
            sum.shortcuts = sum.shortcuts.saturating_add(t.shortcuts);
            sum.trashed = sum.trashed.saturating_add(t.trashed);
            sum.file_bytes = sum.file_bytes.saturating_add(t.file_bytes);
            sum.versions = sum.versions.saturating_add(t.versions);
            sum.version_bytes = sum.version_bytes.saturating_add(t.version_bytes);
            sum.events_1d = sum.events_1d.saturating_add(t.events_1d);
            sum.events_7d = sum.events_7d.saturating_add(t.events_7d);
        }
        Ok(sum)
    }
}
