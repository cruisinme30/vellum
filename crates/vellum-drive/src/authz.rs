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
    SqlStore, Subject, Tuple, TupleRelation,
};
use vellum_kernel::{ObjectId, ShareId, SpaceId, Timestamp, UserId};

use crate::error::{DriveError, DriveResult};
use crate::ports::{Access, DriveIndex, Member, Permission, SpaceRole};
use crate::sharing::{Grant, Grantee, Role, ShareAccess};

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

/// The SQL store reads the tree from `page_ancestors`, which the drive's
/// [`crate::sql::SqlIndex`] keeps in step on every write (D141), so there
/// is nothing to hand over. Pair it with that index on the same database.
impl TreeSync for SqlStore {
    fn set_parents(&self, _space: Id, _parents: BTreeMap<Id, Option<Id>>) -> DriveResult<()> {
        Ok(())
    }
}

/// As for [`SqlStore`]: each shard's `page_ancestors` is kept by the
/// sharded drive index on the same shard.
impl TreeSync for vellum_authz::ShardedStore {
    fn set_parents(&self, _space: Id, _parents: BTreeMap<Id, Option<Id>>) -> DriveResult<()> {
        Ok(())
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

    async fn members(&self, space: SpaceId) -> DriveResult<Vec<Member>> {
        let graph = self
            .authz
            .store()
            .load_space(id(space))
            .await
            .map_err(access_error)?;
        let now = now_ms();
        let mut strongest: BTreeMap<u128, Relation> = BTreeMap::new();
        for tuple in &graph.tuples {
            let (Object::Space(object), TupleRelation::Role(role), Subject::User(user)) =
                (tuple.object, tuple.relation, tuple.subject)
            else {
                continue;
            };
            if object != id(space) || tuple.expires_at.is_some_and(|at| at <= now) {
                continue;
            }
            let entry = strongest.entry(user.0).or_insert(role);
            *entry = (*entry).max(role);
        }
        Ok(strongest
            .into_iter()
            .map(|(user, role)| Member {
                user: UserId::from_u128(user),
                role: space_role(role),
            })
            .collect())
    }

    async fn set_member(
        &self,
        space: SpaceId,
        user: UserId,
        role: Option<SpaceRole>,
    ) -> DriveResult<()> {
        let graph = self
            .authz
            .store()
            .load_space(id(space))
            .await
            .map_err(access_error)?;
        let object = Object::Space(id(space));
        let subject = Subject::User(id(user));
        let mut changes: Vec<Change> = graph
            .tuples
            .iter()
            .filter(|t| t.object == object && t.subject == subject)
            .filter_map(|t| match t.relation {
                TupleRelation::Role(held) if Some(held) != role.map(relation) => {
                    Some(Change::Revoke {
                        object,
                        relation: held,
                        subject,
                    })
                }
                _ => None,
            })
            .collect();
        if let Some(role) = role {
            changes.push(Change::Grant(Tuple::role(object, relation(role), subject)));
        }
        let _ = self
            .authz
            .write(id(space), AuthzActor::System, &changes)
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

const fn space_role(relation: Relation) -> SpaceRole {
    match relation {
        Relation::Viewer => SpaceRole::Viewer,
        Relation::Commenter => SpaceRole::Commenter,
        Relation::Editor => SpaceRole::Editor,
        Relation::Manager => SpaceRole::Manager,
        Relation::Owner => SpaceRole::Owner,
    }
}

const fn relation(role: SpaceRole) -> Relation {
    match role {
        SpaceRole::Viewer => Relation::Viewer,
        SpaceRole::Commenter => Relation::Commenter,
        SpaceRole::Editor => Relation::Editor,
        SpaceRole::Manager => Relation::Manager,
        SpaceRole::Owner => Relation::Owner,
    }
}

fn now_ms() -> i64 {
    vellum_kernel::Timestamp::now().unix_ms()
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

/// A write refused for lack of access reads as permission denied; anything
/// else is a storage failure.
fn write_error(err: vellum_authz::Error) -> DriveError {
    match err {
        vellum_authz::Error::Forbidden { .. } => DriveError::PermissionDenied,
        other => access_error(other),
    }
}

const fn role_relation(role: Role) -> Relation {
    match role {
        Role::Viewer => Relation::Viewer,
        Role::Commenter => Relation::Commenter,
        Role::Editor => Relation::Editor,
        Role::Manager => Relation::Manager,
        Role::Owner => Relation::Owner,
    }
}

const fn relation_role(relation: Relation) -> Role {
    match relation {
        Relation::Viewer => Role::Viewer,
        Relation::Commenter => Role::Commenter,
        Relation::Editor => Role::Editor,
        Relation::Manager => Role::Manager,
        Relation::Owner => Role::Owner,
    }
}

const fn subject(grantee: Grantee) -> Subject {
    match grantee {
        Grantee::User(user) => Subject::User(Id(user.as_u128())),
        Grantee::Group(group) => Subject::Group(Id(group.as_u128())),
    }
}

/// The object a grant on `node` is written to: the space itself for its root.
fn object_of(space: SpaceId, node: ObjectId) -> Object {
    if node.as_u128() == space.as_u128() {
        Object::Space(id(space))
    } else {
        Object::Node(id(node))
    }
}

impl<S: AuthzStore + TreeSync + 'static> AuthzAccess<S> {
    /// The live role tuples on `node`, with their grantees.
    async fn role_tuples(
        &self,
        space: SpaceId,
        node: ObjectId,
    ) -> DriveResult<Vec<(Tuple, Grantee, Role)>> {
        let graph = self
            .authz
            .store()
            .load_space(id(space))
            .await
            .map_err(access_error)?;
        let object = object_of(space, node);
        let now = Timestamp::now().unix_ms();
        Ok(graph
            .tuples
            .into_iter()
            .filter(|t| t.object == object && t.expires_at.is_none_or(|at| at > now))
            .filter_map(|t| {
                let TupleRelation::Role(relation) = t.relation else {
                    return None;
                };
                let grantee = match t.subject {
                    Subject::User(user) => Grantee::User(UserId::from_u128(user.0)),
                    Subject::Group(group) => Grantee::Group(ObjectId::from_u128(group.0)),
                    Subject::ShareLink(_) => return None,
                };
                Some((t, grantee, relation_role(relation)))
            })
            .collect())
    }
}

#[async_trait]
impl<S: AuthzStore + TreeSync + 'static> ShareAccess for AuthzAccess<S> {
    async fn can_share(&self, user: UserId, space: SpaceId, node: ObjectId) -> DriveResult<bool> {
        let decision = self
            .authz
            .check(
                id(space),
                principal(user),
                id(node),
                vellum_authz::Permission::Share,
            )
            .await
            .map_err(access_error)?;
        Ok(decision.allowed)
    }

    async fn grants(&self, space: SpaceId, node: ObjectId) -> DriveResult<Vec<Grant>> {
        Ok(self
            .role_tuples(space, node)
            .await?
            .into_iter()
            .map(|(tuple, grantee, role)| Grant {
                grantee,
                role,
                expires_at: tuple
                    .expires_at
                    .and_then(|ms| Timestamp::from_unix_ms(ms).ok()),
            })
            .collect())
    }

    async fn grant(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        grant: &Grant,
    ) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let object = object_of(space, node);
        let relation = role_relation(grant.role);
        let mut changes: Vec<Change> = self
            .role_tuples(space, node)
            .await?
            .into_iter()
            .filter(|(_, grantee, role)| *grantee == grant.grantee && *role != grant.role)
            .map(|(_, _, role)| Change::Revoke {
                object,
                relation: role_relation(role),
                subject: subject(grant.grantee),
            })
            .collect();
        changes.push(Change::Grant(Tuple {
            object,
            relation: TupleRelation::Role(relation),
            subject: subject(grant.grantee),
            expires_at: grant.expires_at.map(Timestamp::unix_ms),
        }));
        let _ = self
            .authz
            .write(id(space), AuthzActor::User(principal_id(user)), &changes)
            .await
            .map_err(write_error)?;
        Ok(())
    }

    async fn revoke(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        grantee: Grantee,
    ) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let object = object_of(space, node);
        let changes: Vec<Change> = self
            .role_tuples(space, node)
            .await?
            .into_iter()
            .filter(|(_, held_by, _)| *held_by == grantee)
            .map(|(_, _, role)| Change::Revoke {
                object,
                relation: role_relation(role),
                subject: subject(grantee),
            })
            .collect();
        if changes.is_empty() {
            return Ok(());
        }
        let _ = self
            .authz
            .write(id(space), AuthzActor::User(principal_id(user)), &changes)
            .await
            .map_err(write_error)?;
        Ok(())
    }

    async fn grant_link(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
        expires_at: Option<Timestamp>,
    ) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let grant = Tuple {
            object: object_of(space, node),
            relation: TupleRelation::Role(Relation::Viewer),
            subject: Subject::ShareLink(Id(link.as_u128())),
            expires_at: expires_at.map(Timestamp::unix_ms),
        };
        let _ = self
            .authz
            .write(
                id(space),
                AuthzActor::User(principal_id(user)),
                &[Change::Grant(grant)],
            )
            .await
            .map_err(write_error)?;
        Ok(())
    }

    async fn revoke_link(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
    ) -> DriveResult<()> {
        self.sync_tree(space).await?;
        let change = Change::Revoke {
            object: object_of(space, node),
            relation: Relation::Viewer,
            subject: Subject::ShareLink(Id(link.as_u128())),
        };
        let _ = self
            .authz
            .write(id(space), AuthzActor::User(principal_id(user)), &[change])
            .await
            .map_err(write_error)?;
        Ok(())
    }

    async fn link_can_read(
        &self,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
    ) -> DriveResult<bool> {
        let decision = self
            .authz
            .check(
                id(space),
                Principal::ShareLink(Id(link.as_u128())),
                id(node),
                vellum_authz::Permission::Read,
            )
            .await
            .map_err(access_error)?;
        Ok(decision.allowed)
    }
}

const fn principal_id(user: UserId) -> Id {
    Id(user.as_u128())
}
