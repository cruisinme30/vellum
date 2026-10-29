// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The [`Access`] port on the `vellum-authz` `ReBAC` engine.
//!
//! Authorization materialises effective access over the folder tree.
//! The tree comes from the index (`page_ancestors`); stores that keep their
//! own copy of it, like the in-memory one, implement [`TreeSync`] so the
//! adapter can hand them the drive's parent links before re-materialising.
//! Owners are `owner` tuples written as Vellum itself, since a creator has
//! no access yet to grant from.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use vellum_authz::{
    Actor as AuthzActor, Authz, AuthzStore, Change, Id, MemoryStore, Object, Principal, Relation,
    Subject, Tuple, TupleRelation,
};
use vellum_kernel::{ObjectId, SpaceId, UserId};

use crate::error::{DriveError, DriveResult};
use crate::ports::{Access, DriveIndex, Permission};

/// A store that keeps its own copy of each space's folder tree.
pub trait TreeSync: Send + Sync {
    /// Replaces a space's parent links (`None`: top level of the space).
    ///
    /// # Errors
    /// Store failures, as [`DriveError::Access`].
    fn set_parents(&self, space: Id, parents: BTreeMap<Id, Option<Id>>) -> DriveResult<()>;
}

impl TreeSync for MemoryStore {
    fn set_parents(&self, space: Id, parents: BTreeMap<Id, Option<Id>>) -> DriveResult<()> {
        self.set_tree(space, parents).map_err(access_error)
    }
}

/// [`Access`] backed by [`Authz`].
pub struct AuthzAccess<S> {
    authz: Arc<Authz<S>>,
    index: Arc<dyn DriveIndex>,
}

impl<S> fmt::Debug for AuthzAccess<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthzAccess").finish_non_exhaustive()
    }
}

impl<S: AuthzStore + TreeSync> AuthzAccess<S> {
    /// Wraps the engine; `index` supplies the folder tree.
    #[must_use]
    pub fn new(authz: Arc<Authz<S>>, index: Arc<dyn DriveIndex>) -> Self {
        Self { authz, index }
    }

    /// The engine, for sharing writes (grants, revokes, restrictions).
    #[must_use]
    pub fn authz(&self) -> &Authz<S> {
        &self.authz
    }

    /// Hands the store the space's current tree: every node, live or
    /// trashed, under its parent, with links to purged parents cut.
    async fn sync_tree(&self, space: SpaceId) -> DriveResult<()> {
        let nodes = self.index.nodes(space).await?;
        let present: HashSet<ObjectId> = nodes.iter().map(|n| n.id).collect();
        let parents = nodes
            .iter()
            .map(|n| {
                let parent = n
                    .trash
                    .as_ref()
                    .map_or(n.parent, |t| t.original_parent)
                    .filter(|p| present.contains(p));
                (id(n.id), parent.map(id))
            })
            .collect();
        self.authz.store().set_parents(id(space), parents)
    }
}

#[async_trait]
impl<S: AuthzStore + TreeSync + 'static> Access for AuthzAccess<S> {
    async fn check(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        permission: Permission,
    ) -> DriveResult<bool> {
        let permission = match permission {
            Permission::View => vellum_authz::Permission::Read,
            Permission::Edit => vellum_authz::Permission::Edit,
            Permission::Manage => vellum_authz::Permission::Manage,
        };
        let decision = self
            .authz
            .check(id(space), principal(user), id(node), permission)
            .await
            .map_err(access_error)?;
        Ok(decision.allowed)
    }

    async fn spaces(&self, user: UserId) -> DriveResult<Vec<SpaceId>> {
        let spaces = self
            .authz
            .spaces(principal(user))
            .await
            .map_err(access_error)?;
        Ok(spaces
            .into_iter()
            .map(|s| SpaceId::from_u128(s.0))
            .collect())
    }

    async fn accessible(&self, user: UserId, space: SpaceId) -> DriveResult<Vec<ObjectId>> {
        let ids = self
            .authz
            .accessible(id(space), principal(user), vellum_authz::Permission::Read)
            .await
            .map_err(access_error)?;
        Ok(ids
            .into_iter()
            .filter(|i| i.0 != space.as_u128())
            .map(|i| ObjectId::from_u128(i.0))
            .collect())
    }

    async fn tree_changed(&self, space: SpaceId, nodes: &[ObjectId]) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let roots: Vec<Id> = nodes.iter().map(|n| id(*n)).collect();
        let _ = self
            .authz
            .tree_changed(id(space), &roots)
            .await
            .map_err(access_error)?;
        Ok(())
    }

    async fn grant_owner(&self, user: UserId, space: SpaceId, node: ObjectId) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let object = if node.as_u128() == space.as_u128() {
            Object::Space(id(space))
        } else {
            Object::Node(id(node))
        };
        let grant = Tuple {
            object,
            relation: TupleRelation::Role(Relation::Owner),
            subject: Subject::User(id(user)),
            expires_at: None,
        };
        let _ = self
            .authz
            .write(id(space), AuthzActor::System, &[Change::Grant(grant)])
            .await
            .map_err(access_error)?;
        Ok(())
    }

    async fn groups_of(&self, user: UserId) -> DriveResult<BTreeSet<ObjectId>> {
        let groups = self
            .authz
            .groups_of(principal(user))
            .await
            .map_err(access_error)?;
        Ok(groups
            .into_iter()
            .map(|g| ObjectId::from_u128(g.0))
            .collect())
    }
}

fn id(value: impl Into<ObjectId>) -> Id {
    Id(value.into().as_u128())
}

const fn principal(user: UserId) -> Principal {
    Principal::User(Id(user.as_u128()))
}

fn access_error(err: impl fmt::Display) -> DriveError {
    DriveError::Access(err.to_string())
}
