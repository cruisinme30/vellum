// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! In-memory adapters for the drive's ports.
//!
//! [`MemoryIndex`] is a complete [`DriveIndex`]: the reference
//! implementation the SQL adapters are tested against, and enough for tests
//! and short-lived tools. [`MemoryAccess`] is a small role-based
//! [`Access`] with folder inheritance, for tests and single-user setups.

// Each method holds the lock for one short, await-free block.
#![allow(
    clippy::significant_drop_tightening,
    reason = "locks are held for single await-free blocks"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use vellum_kernel::{ObjectId, SpaceId, Timestamp, UserId, VersionId};
use vellum_store::BlobPath;

use crate::error::DriveResult;
use crate::model::{Activity, DriveTotals, Node, Space, VersionRecord};
use crate::ports::{Access, DriveIndex, Member, Permission, SpaceRole};

#[derive(Debug, Default)]
struct State {
    spaces: BTreeMap<SpaceId, Space>,
    nodes: HashMap<ObjectId, Node>,
    stars: HashMap<UserId, HashMap<ObjectId, Timestamp>>,
    recents: HashMap<UserId, HashMap<ObjectId, Timestamp>>,
    activity: Vec<Activity>,
    links: HashMap<ObjectId, Vec<ObjectId>>,
    versions: BTreeMap<VersionId, VersionRecord>,
}

/// A [`DriveIndex`] held in memory.
#[derive(Debug, Default)]
pub struct MemoryIndex {
    state: Mutex<State>,
}

impl MemoryIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave State half-written
        // (every method mutates with single inserts), so recover the data.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The parent chain of a node, innermost first, ending at the space.
    #[must_use]
    pub fn chain(&self, node: ObjectId) -> Vec<ObjectId> {
        let state = self.lock();
        let mut chain = vec![node];
        let mut current = state.nodes.get(&node);
        while let Some(n) = current {
            match n.parent {
                Some(parent) if !chain.contains(&parent) => {
                    chain.push(parent);
                    current = state.nodes.get(&parent);
                }
                _ => {
                    chain.push(n.space.into());
                    break;
                }
            }
        }
        chain
    }
}

#[async_trait]
impl DriveIndex for MemoryIndex {
    async fn put_space(&self, space: &Space) -> DriveResult<()> {
        let _ = self.lock().spaces.insert(space.id, space.clone());
        Ok(())
    }

    async fn space(&self, id: SpaceId) -> DriveResult<Option<Space>> {
        Ok(self.lock().spaces.get(&id).cloned())
    }

    async fn spaces(&self) -> DriveResult<Vec<Space>> {
        Ok(self.lock().spaces.values().cloned().collect())
    }

    async fn put_node(&self, node: &Node) -> DriveResult<()> {
        let _ = self.lock().nodes.insert(node.id, node.clone());
        Ok(())
    }

    async fn node(&self, id: ObjectId) -> DriveResult<Option<Node>> {
        Ok(self.lock().nodes.get(&id).cloned())
    }

    async fn node_at(
        &self,
        space: SpaceId,
        path: &BlobPath,
    ) -> DriveResult<Option<Node>> {
        Ok(self
            .lock()
            .nodes
            .values()
            .find(|n| n.space == space && &n.path == path)
            .cloned())
    }

    async fn children(
        &self,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<Vec<Node>> {
        let mut out: Vec<Node> = self
            .lock()
            .nodes
            .values()
            .filter(|n| n.space == space && n.parent == parent && n.is_live())
            .cloned()
            .collect();
        out.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn nodes(&self, space: SpaceId) -> DriveResult<Vec<Node>> {
        let mut out: Vec<Node> = self
            .lock()
            .nodes
            .values()
            .filter(|n| n.space == space)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    async fn by_title(
        &self,
        space: SpaceId,
        title: &str,
    ) -> DriveResult<Vec<Node>> {
        Ok(self
            .lock()
            .nodes
            .values()
            .filter(|n| n.space == space && n.is_live() && n.title == title)
            .cloned()
            .collect())
    }

    async fn remove_node(&self, id: ObjectId) -> DriveResult<()> {
        let mut state = self.lock();
        let _ = state.nodes.remove(&id);
        let _ = state.links.remove(&id);
        for stars in state.stars.values_mut() {
            let _ = stars.remove(&id);
        }
        for recents in state.recents.values_mut() {
            let _ = recents.remove(&id);
        }
        state.versions.retain(|_, v| v.doc != id);
        Ok(())
    }

    async fn set_starred(
        &self,
        user: UserId,
        node: ObjectId,
        at: Option<Timestamp>,
    ) -> DriveResult<()> {
        let mut state = self.lock();
        let stars = state.stars.entry(user).or_default();
        match at {
            Some(at) => {
                let _ = stars.insert(node, at);
            }
            None => {
                let _ = stars.remove(&node);
            }
        }
        Ok(())
    }

    async fn starred(&self, user: UserId) -> DriveResult<Vec<ObjectId>> {
        let state = self.lock();
        let mut out: Vec<(ObjectId, Timestamp)> = state
            .stars
            .get(&user)
            .map(|s| s.iter().map(|(id, at)| (*id, *at)).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        Ok(out.into_iter().map(|(id, _)| id).collect())
    }

    async fn touch(
        &self,
        user: UserId,
        node: ObjectId,
        at: Timestamp,
    ) -> DriveResult<()> {
        let _ = self
            .lock()
            .recents
            .entry(user)
            .or_default()
            .insert(node, at);
        Ok(())
    }

    async fn recent(
        &self,
        user: UserId,
        limit: usize,
    ) -> DriveResult<Vec<(ObjectId, Timestamp)>> {
        let state = self.lock();
        let mut out: Vec<(ObjectId, Timestamp)> = state
            .recents
            .get(&user)
            .map(|s| s.iter().map(|(id, at)| (*id, *at)).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        out.truncate(limit);
        Ok(out)
    }

    async fn append_activity(&self, entry: &Activity) -> DriveResult<()> {
        self.lock().activity.push(entry.clone());
        Ok(())
    }

    async fn activity(
        &self,
        node: ObjectId,
        limit: usize,
    ) -> DriveResult<Vec<Activity>> {
        Ok(self
            .lock()
            .activity
            .iter()
            .rev()
            .filter(|a| a.node == node || a.ancestors.contains(&node))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn set_links(
        &self,
        source: ObjectId,
        targets: &[ObjectId],
    ) -> DriveResult<()> {
        let mut unique: Vec<ObjectId> = targets.to_vec();
        unique.sort_unstable();
        unique.dedup();
        let _ = self.lock().links.insert(source, unique);
        Ok(())
    }

    async fn backlinks(&self, target: ObjectId) -> DriveResult<Vec<ObjectId>> {
        let mut out: Vec<ObjectId> = self
            .lock()
            .links
            .iter()
            .filter(|(source, targets)| {
                **source != target && targets.contains(&target)
            })
            .map(|(source, _)| *source)
            .collect();
        out.sort_unstable();
        Ok(out)
    }

    async fn put_version(&self, version: &VersionRecord) -> DriveResult<()> {
        let _ = self.lock().versions.insert(version.id, version.clone());
        Ok(())
    }

    async fn versions(&self, doc: ObjectId) -> DriveResult<Vec<VersionRecord>> {
        Ok(self
            .lock()
            .versions
            .values()
            .rev()
            .filter(|v| v.doc == doc)
            .cloned()
            .collect())
    }

    async fn remove_version(&self, id: VersionId) -> DriveResult<()> {
        let _ = self.lock().versions.remove(&id);
        Ok(())
    }

    async fn usage(&self, space: SpaceId) -> DriveResult<u64> {
        let state = self.lock();
        let nodes: u64 = state
            .nodes
            .values()
            .filter(|n| n.space == space)
            .map(|n| n.size)
            .sum();
        let versions: u64 = state
            .versions
            .values()
            .filter(|v| v.space == space)
            .map(|v| v.size)
            .sum();
        Ok(nodes.saturating_add(versions))
    }

    async fn totals(&self, now: Timestamp) -> DriveResult<DriveTotals> {
        let state = self.lock();
        let mut totals = DriveTotals {
            spaces: count(state.spaces.len()),
            versions: count(state.versions.len()),
            version_bytes: state.versions.values().map(|v| v.size).sum(),
            ..DriveTotals::default()
        };
        for node in state.nodes.values() {
            totals.add_node(node.kind, node.is_live(), node.size);
        }
        let day = DAY_MS;
        let ago = |a: &Activity| now.unix_ms().saturating_sub(a.at.unix_ms());
        totals.events_1d =
            count(state.activity.iter().filter(|a| ago(a) <= day).count());
        totals.events_7d =
            count(state.activity.iter().filter(|a| ago(a) <= 7 * day).count());
        Ok(totals)
    }
}

/// A day in milliseconds.
const DAY_MS: i64 = 86_400_000;

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// A role a person holds on a space or node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// May view.
    Viewer,
    /// May view and edit.
    Editor,
    /// May do anything, including purge and settings.
    Manager,
}

impl Role {
    const fn allows(self, permission: Permission) -> bool {
        match permission {
            Permission::View => true,
            Permission::Edit => matches!(self, Self::Editor | Self::Manager),
            Permission::Manage => matches!(self, Self::Manager),
        }
    }
}

/// Role grants on spaces and nodes, inherited down the folder tree of any
/// [`DriveIndex`]. Deny by default.
#[derive(Debug)]
pub struct MemoryAccess {
    index: Arc<dyn DriveIndex>,
    grants: Mutex<HashMap<(ObjectId, UserId), Role>>,
    groups: Mutex<BTreeMap<UserId, BTreeSet<ObjectId>>>,
}

impl MemoryAccess {
    /// Grants nothing to anyone yet.
    #[must_use]
    pub fn new(index: Arc<dyn DriveIndex>) -> Self {
        Self {
            index,
            grants: Mutex::new(HashMap::new()),
            groups: Mutex::new(BTreeMap::new()),
        }
    }

    /// Grants `role` on a node, or on a whole space via its ID.
    pub fn grant(&self, user: UserId, node: impl Into<ObjectId>, role: Role) {
        let _ = self.lock().insert((node.into(), user), role);
    }

    /// Makes `user` a member of `group`.
    pub fn join(&self, user: UserId, group: ObjectId) {
        let _ = self
            .groups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(user)
            .or_default()
            .insert(group);
    }

    /// Removes a grant made on `node` itself.
    pub fn revoke(&self, user: UserId, node: impl Into<ObjectId>) {
        let _ = self.lock().remove(&(node.into(), user));
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(ObjectId, UserId), Role>> {
        self.grants.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The parent chain of a node, innermost first, ending at the space.
    async fn chain(&self, node: ObjectId) -> DriveResult<Vec<ObjectId>> {
        let mut chain = vec![node];
        let mut current = self.index.node(node).await?;
        while let Some(n) = current {
            match n.parent {
                Some(parent) if !chain.contains(&parent) => {
                    chain.push(parent);
                    current = self.index.node(parent).await?;
                }
                _ => {
                    chain.push(n.space.into());
                    break;
                }
            }
        }
        Ok(chain)
    }

    async fn role(
        &self,
        user: UserId,
        node: ObjectId,
    ) -> DriveResult<Option<Role>> {
        let chain = self.chain(node).await?;
        let grants = self.lock();
        Ok(chain
            .iter()
            .filter_map(|id| grants.get(&(*id, user)).copied())
            .max())
    }
}

#[async_trait]
impl Access for MemoryAccess {
    async fn check(
        &self,
        user: UserId,
        _space: SpaceId,
        node: ObjectId,
        permission: Permission,
    ) -> DriveResult<bool> {
        Ok(self
            .role(user, node)
            .await?
            .is_some_and(|role| role.allows(permission)))
    }

    async fn spaces(&self, user: UserId) -> DriveResult<Vec<SpaceId>> {
        let granted: Vec<ObjectId> = self
            .lock()
            .keys()
            .filter(|(_, u)| *u == user)
            .map(|(id, _)| *id)
            .collect();
        let mut spaces = HashSet::new();
        for id in granted {
            if let Some(node) = self.index.node(id).await? {
                let _ = spaces.insert(node.space);
            } else if self.index.space(id.into()).await?.is_some() {
                let _ = spaces.insert(SpaceId::from(id));
            }
        }
        let mut out: Vec<SpaceId> = spaces.into_iter().collect();
        out.sort_unstable();
        Ok(out)
    }

    async fn accessible(
        &self,
        user: UserId,
        space: SpaceId,
    ) -> DriveResult<Vec<ObjectId>> {
        let mut out = Vec::new();
        for node in self.index.nodes(space).await? {
            if self.role(user, node.id).await?.is_some() {
                out.push(node.id);
            }
        }
        Ok(out)
    }

    async fn tree_changed(
        &self,
        _space: SpaceId,
        _nodes: &[ObjectId],
    ) -> DriveResult<()> {
        // Inheritance is computed on every check from the live tree.
        Ok(())
    }

    async fn grant_owner(
        &self,
        user: UserId,
        _space: SpaceId,
        node: ObjectId,
    ) -> DriveResult<()> {
        self.grant(user, node, Role::Manager);
        Ok(())
    }

    async fn members(&self, space: SpaceId) -> DriveResult<Vec<Member>> {
        let root = ObjectId::from(space);
        let mut out: Vec<Member> = self
            .lock()
            .iter()
            .filter(|((node, _), _)| *node == root)
            .map(|((_, user), role)| Member {
                user: *user,
                role: match role {
                    Role::Viewer => SpaceRole::Viewer,
                    Role::Editor => SpaceRole::Editor,
                    Role::Manager => SpaceRole::Manager,
                },
            })
            .collect();
        out.sort_by_key(|m| m.user);
        Ok(out)
    }

    async fn set_member(
        &self,
        space: SpaceId,
        user: UserId,
        role: Option<SpaceRole>,
    ) -> DriveResult<()> {
        let root = ObjectId::from(space);
        match role {
            None => self.revoke(user, root),
            Some(SpaceRole::Viewer | SpaceRole::Commenter) => {
                self.grant(user, root, Role::Viewer)
            }
            Some(SpaceRole::Editor) => self.grant(user, root, Role::Editor),
            Some(SpaceRole::Manager | SpaceRole::Owner) => {
                self.grant(user, root, Role::Manager)
            }
        }
        Ok(())
    }

    async fn groups_of(&self, user: UserId) -> DriveResult<BTreeSet<ObjectId>> {
        Ok(self
            .groups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&user)
            .cloned()
            .unwrap_or_default())
    }
}
