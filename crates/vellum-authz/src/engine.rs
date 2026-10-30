// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The authorization service: checks on the read path, tuple writes and
//! re-materialisation on the write path.
//!
//! Checks never compute anything: they read one `effective_access` row.
//! Every write recomputes the affected rows and bumps the space's authz
//! epoch in the same transaction, so a permission change is visible to the
//! next check on any node (well inside the 1 s target).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{Instrument as _, Span, field};

use crate::graph::{self, Directory, Materialised, SpaceGraph};
use crate::model::{
    Id, Object, Permission, Principal, Relation, Subject, Tuple, TupleRelation,
};
use crate::store::{
    AuthzStore, Commit, EffectiveRow, Scope, StoreError, TupleKey,
};

/// How many times a write retries after losing an epoch race.
const MAX_ATTEMPTS: usize = 8;

/// Who is making a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// The server itself: bootstrap, reindex, import, the expiry sweep.
    /// Never derived from a request.
    System,
    /// A signed-in person; their own access is checked first.
    User(Id),
}

/// One change to a space's tuples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Grant a role on the space or a node (replacing any existing grant
    /// of that role to that subject, so an expiry can be changed).
    Grant(Tuple),
    /// Remove a role grant.
    Revoke {
        /// The space or node.
        object: Object,
        /// The role.
        relation: Relation,
        /// Who loses it.
        subject: Subject,
    },
    /// Mark a node as narrowing inherited access.
    Restrict(Id),
    /// Remove the narrowing mark.
    Unrestrict(Id),
}

impl Change {
    const fn object(&self) -> Object {
        match self {
            Self::Grant(t) => t.object,
            Self::Revoke { object, .. } => *object,
            Self::Restrict(n) | Self::Unrestrict(n) => Object::Node(*n),
        }
    }

    /// The relation the actor needs on the object to make this change.
    fn needs(&self) -> Relation {
        match self {
            Self::Grant(Tuple {
                relation: TupleRelation::Role(r),
                ..
            })
            | Self::Revoke { relation: r, .. } => (*r).max(Relation::Manager),
            Self::Grant(_) | Self::Restrict(_) | Self::Unrestrict(_) => {
                Relation::Manager
            }
        }
    }
}

/// The answer to a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// Whether the permission is granted.
    pub allowed: bool,
    /// The principal's relation on the object, if any.
    pub relation: Option<Relation>,
    /// The authz epoch the answer was computed at, if a row was found.
    pub epoch: Option<u64>,
}

/// Why an authorization call failed. A denied check is not an error; it is
/// a [`Decision`] with `allowed: false`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The actor may not make this change.
    #[error("{actor:?} may not change access on {object:?}")]
    Forbidden {
        /// Who tried.
        actor: Actor,
        /// What they tried to change.
        object: Object,
    },
    /// The change does not fit the model, for example a role on a group.
    #[error("malformed change: {0:?}")]
    Malformed(Change),
    /// The change names a node that is not in the space.
    #[error("node {node} is not in space {space}")]
    UnknownNode {
        /// The space.
        space: Id,
        /// The missing node.
        node: Id,
    },
    /// The change names a different space.
    #[error("object {object:?} does not belong to space {space}")]
    WrongSpace {
        /// The space written to.
        space: Id,
        /// The object.
        object: Object,
    },
    /// Concurrent writers kept winning the epoch race.
    #[error("gave up after {MAX_ATTEMPTS} conflicting writes to space {0}")]
    Contended(Id),
    /// The folder tree is not a forest.
    #[error(transparent)]
    Graph(#[from] graph::GraphError),
    /// The index failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Milliseconds since the Unix epoch, injected so tests control time.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The authorization service over a storage adapter.
pub struct Authz<S> {
    store: S,
    clock: Clock,
}

impl<S> std::fmt::Debug for Authz<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authz").finish_non_exhaustive()
    }
}

impl<S: AuthzStore> Authz<S> {
    /// A service on the wall clock.
    pub fn new(store: S) -> Self {
        Self::with_clock(store, Arc::new(system_clock))
    }

    /// A service with an injected clock.
    pub fn with_clock(store: S, clock: Clock) -> Self {
        Self { store, clock }
    }

    /// The storage adapter.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Whether `principal` may do `permission` on `object` (the space or a
    /// node of it). One indexed read; deny when no row exists.
    ///
    /// # Errors
    ///
    /// Fails only if the index cannot be read.
    pub async fn check(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
        permission: Permission,
    ) -> Result<Decision, Error> {
        let span = tracing::info_span!(
            "authz.check",
            authz.space = %space,
            authz.object = %object,
            authz.permission = permission.as_str(),
            authz.allowed = field::Empty,
            authz.relation = field::Empty,
            authz.epoch = field::Empty,
        );
        async {
            let rows = self
                .store
                .lookup(space, principal, object, (self.clock)())
                .await?;
            let row = rows.into_iter().max();
            let decision = Decision {
                allowed: row
                    .is_some_and(|(r, _)| r.implies(permission.required())),
                relation: row.map(|(r, _)| r),
                epoch: row.map(|(_, e)| e),
            };
            let span = Span::current();
            let _ = span.record("authz.allowed", decision.allowed);
            if let Some(r) = decision.relation {
                let _ = span.record("authz.relation", r.as_str());
            }
            if let Some(e) = decision.epoch {
                let _ = span.record("authz.epoch", e);
            }
            tracing::debug!(
                monotonic_counter.vellum.authz.checks = 1_u64,
                allowed = decision.allowed,
                "authz check"
            );
            Ok(decision)
        }
        .instrument(span)
        .await
    }

    /// The objects in a space `principal` may do `permission` on, for
    /// filtering search results, trees and MCP lists. One indexed read.
    ///
    /// # Errors
    ///
    /// Fails only if the index cannot be read.
    pub async fn accessible(
        &self,
        space: Id,
        principal: Principal,
        permission: Permission,
    ) -> Result<Vec<Id>, Error> {
        let span = tracing::info_span!(
            "authz.list",
            authz.space = %space,
            authz.permission = permission.as_str(),
            authz.rows = field::Empty,
        );
        async {
            let rows =
                self.store.list(space, principal, (self.clock)()).await?;
            let ids: Vec<Id> = rows
                .into_iter()
                .filter(|(_, r)| r.implies(permission.required()))
                .map(|(id, _)| id)
                .collect::<BTreeSet<Id>>()
                .into_iter()
                .collect();
            let _ = Span::current().record("authz.rows", ids.len());
            Ok(ids)
        }
        .instrument(span)
        .await
    }

    /// Which of `objects` (in any space) `principal` may do `permission`
    /// on: an exact batch check for search and MCP results, deny by
    /// default. One indexed read.
    ///
    /// # Errors
    ///
    /// Fails only if the index cannot be read.
    pub async fn filter(
        &self,
        principal: Principal,
        permission: Permission,
        objects: &[Id],
    ) -> Result<BTreeSet<Id>, Error> {
        let span = tracing::info_span!(
            "authz.filter",
            authz.permission = permission.as_str(),
            authz.asked = objects.len(),
            authz.allowed = field::Empty,
        );
        async {
            let rows = self
                .store
                .lookup_many(principal, objects, (self.clock)())
                .await?;
            let allowed: BTreeSet<Id> = rows
                .into_iter()
                .filter(|(_, r)| r.implies(permission.required()))
                .map(|(id, _)| id)
                .collect();
            let _ = Span::current().record("authz.allowed", allowed.len());
            Ok(allowed)
        }
        .instrument(span)
        .await
    }

    /// Every space in which `principal` can read at least one object, to
    /// scope a search before it runs.
    ///
    /// # Errors
    ///
    /// Fails only if the index cannot be read.
    pub async fn spaces(&self, principal: Principal) -> Result<Vec<Id>, Error> {
        let span = tracing::info_span!("authz.spaces");
        async { Ok(self.store.spaces_of(principal, (self.clock)()).await?) }
            .instrument(span)
            .await
    }

    /// Every group `principal` belongs to right now, including groups
    /// reached through nested membership. Expired memberships do not count.
    /// For callers that match `group:<id>` rules outside the engine, such
    /// as book-section restrictions in `book.yaml`.
    ///
    /// # Errors
    ///
    /// Fails only if the index cannot be read.
    pub async fn groups_of(
        &self,
        principal: Principal,
    ) -> Result<BTreeSet<Id>, Error> {
        let span =
            tracing::info_span!("authz.groups_of", authz.groups = field::Empty);
        async {
            let directory = Directory::new(
                &self.store.load_directory().await?,
                (self.clock)(),
            );
            let groups = directory.groups_of(principal);
            let _ = Span::current().record("authz.groups", groups.len());
            Ok(groups)
        }
        .instrument(span)
        .await
    }

    /// The space's current authz epoch. Caches keyed on it (render, search,
    /// MCP resources) are invalidated by any permission change.
    ///
    /// # Errors
    ///
    /// Fails if the space is unknown or the index cannot be read.
    pub async fn epoch(&self, space: Id) -> Result<u64, Error> {
        Ok(self.store.epoch(space).await?)
    }

    /// Applies tuple changes to one space and re-materialises the affected
    /// subtrees, atomically. Returns the new epoch.
    ///
    /// A user actor needs `manager` on each changed object (`owner` to
    /// grant or revoke `owner`), so nobody can hand out more than they
    /// hold.
    ///
    /// # Errors
    ///
    /// [`Error::Forbidden`] when the actor lacks access; [`Error::Malformed`],
    /// [`Error::WrongSpace`] or [`Error::UnknownNode`] for a bad change;
    /// [`Error::Contended`] when concurrent writers keep winning.
    pub async fn write(
        &self,
        space: Id,
        actor: Actor,
        changes: &[Change],
    ) -> Result<u64, Error> {
        let span = tracing::info_span!(
            "authz.write",
            authz.space = %space,
            authz.changes = changes.len(),
            authz.epoch = field::Empty,
        );
        async {
            for change in changes {
                self.authorise(space, actor, change).await?;
            }
            let (upsert, delete) = tuple_writes(changes)?;
            let epoch = self
                .recompute(space, upsert, delete, |graph| {
                    validate(graph, changes)?;
                    Ok(Some(changed_scope(graph, changes)))
                })
                .await?;
            let _ = Span::current().record("authz.epoch", epoch);
            tracing::info!(
                monotonic_counter.vellum.authz.writes = 1_u64,
                epoch,
                "access changed"
            );
            Ok(epoch)
        }
        .instrument(span)
        .await
    }

    /// Re-materialises after the index moved, added or removed nodes
    /// (a reindex, a move, a trash). Pass the roots of every changed
    /// subtree; IDs no longer in the space have their rows removed.
    ///
    /// # Errors
    ///
    /// Fails if the tree is not a forest or the index fails.
    pub async fn tree_changed(
        &self,
        space: Id,
        roots: &[Id],
    ) -> Result<u64, Error> {
        let span = tracing::info_span!(
            "authz.tree_changed",
            authz.space = %space,
            authz.roots = roots.len()
        );
        self.recompute(space, Vec::new(), Vec::new(), |graph| {
            let mut objects = BTreeSet::new();
            for &root in roots {
                if graph.parents.contains_key(&root) {
                    objects.extend(graph.subtree(root));
                } else {
                    let _ = objects.insert(root);
                }
            }
            Ok(Some(Scope::Objects(objects)))
        })
        .instrument(span)
        .await
    }

    /// Re-materialises a whole space, dropping rows for nodes that no
    /// longer exist. Run after `vellum reindex`.
    ///
    /// # Errors
    ///
    /// Fails if the tree is not a forest or the index fails.
    pub async fn rebuild(&self, space: Id) -> Result<u64, Error> {
        let span = tracing::info_span!("authz.rebuild", authz.space = %space);
        self.recompute(space, Vec::new(), Vec::new(), |_| Ok(Some(Scope::All)))
            .instrument(span)
            .await
    }

    /// Garbage-collects expired tuples: the space's grants and every
    /// expired group membership, then re-materialises the space to drop
    /// dead rows. Access is already exact without it (rows carry their
    /// expiry); this only reclaims space. Returns when something next
    /// expires, so a job can schedule the next run.
    ///
    /// # Errors
    ///
    /// Fails if the index fails.
    pub async fn sweep_expired(&self, space: Id) -> Result<Option<i64>, Error> {
        let span = tracing::info_span!("authz.sweep", authz.space = %space);
        async {
            let now = (self.clock)();
            let graph = self.store.load_space(space).await?;
            let expired: Vec<TupleKey> = graph
                .tuples
                .iter()
                .filter(|t| !t.is_live(now))
                .map(Tuple::key)
                .collect();
            let members = self.store.load_directory().await?;
            let dead_members: Vec<TupleKey> = members
                .iter()
                .filter(|t| !t.is_live(now))
                .map(Tuple::key)
                .collect();
            if !dead_members.is_empty() {
                self.store.write_directory(Vec::new(), dead_members).await?;
            }
            let _ = self
                .recompute(space, Vec::new(), expired, |_| Ok(Some(Scope::All)))
                .await?;
            Ok(graph::next_expiry(graph.tuples.iter().chain(&members), now))
        }
        .instrument(span)
        .await
    }

    /// Adds or removes group members, then re-materialises every space
    /// that grants to the group or a group containing it.
    ///
    /// Group administration is an admin action: the caller checks the
    /// admin role (with re-authentication) before calling this.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] for a tuple that is not a membership; otherwise
    /// index failures.
    pub async fn update_group(
        &self,
        group: Id,
        add: &[Subject],
        remove: &[Subject],
    ) -> Result<(), Error> {
        let span = tracing::info_span!(
            "authz.group",
            authz.group = %group,
            authz.spaces = field::Empty
        );
        async {
            let upsert: Vec<Tuple> =
                add.iter().map(|s| Tuple::member(group, *s)).collect();
            let delete: Vec<TupleKey> = remove
                .iter()
                .map(|s| Tuple::member(group, *s).key())
                .collect();
            // Groups containing this one, before and after, so removing a
            // nested group also refreshes spaces that granted the outer one.
            let before = Directory::new(
                &self.store.load_directory().await?,
                (self.clock)(),
            );
            self.store.write_directory(upsert, delete).await?;
            let after = Directory::new(
                &self.store.load_directory().await?,
                (self.clock)(),
            );
            let mut groups = before.containing(group);
            groups.extend(after.containing(group));
            let spaces = self.store.spaces_granting(&groups).await?;
            let _ = Span::current().record("authz.spaces", spaces.len());
            for space in spaces {
                let _ = self.rebuild(space).await?;
            }
            Ok(())
        }
        .instrument(span)
        .await
    }

    async fn authorise(
        &self,
        space: Id,
        actor: Actor,
        change: &Change,
    ) -> Result<(), Error> {
        let Actor::User(user) = actor else {
            return Ok(());
        };
        let object = change.object();
        let object_id = match object {
            Object::Space(id) | Object::Node(id) | Object::Group(id) => id,
        };
        let held = self
            .store
            .lookup(space, Principal::User(user), object_id, (self.clock)())
            .await?;
        if held.iter().any(|(r, _)| r.implies(change.needs())) {
            Ok(())
        } else {
            tracing::info!(
                monotonic_counter.vellum.authz.forbidden_writes = 1_u64,
                "access change refused"
            );
            Err(Error::Forbidden { actor, object })
        }
    }

    /// Loads, applies `upsert`/`delete` in memory, materialises, and
    /// commits the rows in `scope`, retrying on epoch conflicts.
    async fn recompute(
        &self,
        space: Id,
        upsert: Vec<Tuple>,
        delete: Vec<TupleKey>,
        scope: impl Fn(&SpaceGraph) -> Result<Option<Scope>, Error>,
    ) -> Result<u64, Error> {
        for attempt in 1..=MAX_ATTEMPTS {
            let mut graph = self.store.load_space(space).await?;
            let Some(scope) = scope(&graph)? else {
                return Ok(graph.epoch);
            };
            apply(&mut graph, &upsert, &delete);
            let now = (self.clock)();
            let directory =
                Directory::new(&self.store.load_directory().await?, now);
            let rows = {
                let span = tracing::debug_span!(
                    "authz.materialise",
                    authz.rows = field::Empty
                );
                let _entered = span.enter();
                let materialised = graph::materialise(&graph, &directory, now)?;
                let rows = rows_in(&graph, &materialised, &scope, now);
                let _ = span.record("authz.rows", rows.len());
                rows
            };
            tracing::debug!(
                histogram.vellum.authz.materialised_rows = rows.len() as u64,
                attempt,
                "materialised"
            );
            let commit = Commit {
                space,
                expected_epoch: graph.epoch,
                upsert: upsert.clone(),
                delete: delete.clone(),
                scope,
                rows,
            };
            match self.store.commit(commit).await {
                Ok(epoch) => return Ok(epoch),
                Err(StoreError::Conflict) => {
                    tracing::debug!(
                        monotonic_counter.vellum.authz.epoch_conflicts = 1_u64,
                        attempt,
                        "epoch conflict, retrying"
                    );
                }
                Err(other) => return Err(other.into()),
            }
        }
        Err(Error::Contended(space))
    }
}

fn tuple_writes(
    changes: &[Change],
) -> Result<(Vec<Tuple>, Vec<TupleKey>), Error> {
    let mut upsert = Vec::new();
    let mut delete = Vec::new();
    for change in changes {
        match *change {
            Change::Grant(t) => {
                if !t.is_well_formed()
                    || !matches!(t.relation, TupleRelation::Role(_))
                {
                    return Err(Error::Malformed(*change));
                }
                upsert.push(t);
            }
            Change::Revoke {
                object,
                relation,
                subject,
            } => {
                if matches!(object, Object::Group(_)) {
                    return Err(Error::Malformed(*change));
                }
                delete.push((object, TupleRelation::Role(relation), subject));
            }
            Change::Restrict(n) => upsert.push(Tuple::restricted(n)),
            Change::Unrestrict(n) => delete.push(Tuple::restricted(n).key()),
        }
    }
    Ok((upsert, delete))
}

fn validate(graph: &SpaceGraph, changes: &[Change]) -> Result<(), Error> {
    for change in changes {
        match change.object() {
            Object::Space(id) if id != graph.space => {
                return Err(Error::WrongSpace {
                    space: graph.space,
                    object: change.object(),
                });
            }
            Object::Node(node) if !graph.parents.contains_key(&node) => {
                return Err(Error::UnknownNode {
                    space: graph.space,
                    node,
                });
            }
            Object::Group(_) => return Err(Error::Malformed(*change)),
            _ => {}
        }
    }
    Ok(())
}

/// The objects whose rows a set of changes can affect: the whole space for
/// a space grant, otherwise each changed node's subtree.
fn changed_scope(graph: &SpaceGraph, changes: &[Change]) -> Scope {
    let mut objects = BTreeSet::new();
    for change in changes {
        match change.object() {
            Object::Space(_) => return Scope::All,
            Object::Node(n) => objects.extend(graph.subtree(n)),
            Object::Group(_) => {}
        }
    }
    Scope::Objects(objects)
}

fn apply(graph: &mut SpaceGraph, upsert: &[Tuple], delete: &[TupleKey]) {
    let gone: BTreeSet<TupleKey> = delete
        .iter()
        .copied()
        .chain(upsert.iter().map(Tuple::key))
        .collect();
    graph.tuples.retain(|t| !gone.contains(&t.key()));
    graph.tuples.extend_from_slice(upsert);
}

fn rows_in(
    graph: &SpaceGraph,
    m: &Materialised,
    scope: &Scope,
    now: i64,
) -> Vec<EffectiveRow> {
    let wanted = |object: &Id| match scope {
        Scope::All => true,
        Scope::Objects(set) => set.contains(object),
    };
    let space_rows = std::iter::once((graph.space, &m.space));
    space_rows
        .chain(m.nodes.iter().map(|(id, a)| (*id, a)))
        .filter(|(object, _)| wanted(object))
        .flat_map(|(object, access)| {
            access.iter().flat_map(move |(principal, levels)| {
                levels.rows(now).into_iter().map(
                    move |(relation, valid_until)| EffectiveRow {
                        object,
                        principal: *principal,
                        relation,
                        valid_until,
                    },
                )
            })
        })
        .collect()
}
