// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An in-memory [`AuthzStore`]: the reference adapter for tests.

// Each method holds the lock for its whole body on purpose: it is the
// transaction boundary that the SQL adapters get from the database.
#![allow(clippy::significant_drop_tightening)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::graph::SpaceGraph;
use crate::model::{Id, Principal, Relation, Subject, Tuple};
use crate::store::{AuthzStore, Commit, Scope, StoreError, TupleKey};

#[derive(Debug, Default)]
struct Space {
    epoch: u64,
    parents: BTreeMap<Id, Option<Id>>,
    tuples: BTreeMap<TupleKey, Tuple>,
    /// `(principal id, object, relation) → (epoch, valid_until)`, the
    /// shape of the `effective_access` primary key.
    effective: BTreeMap<(Id, Id, Relation), (u64, Option<i64>)>,
}

#[derive(Debug, Default)]
struct State {
    spaces: BTreeMap<Id, Space>,
    directory: BTreeMap<TupleKey, Tuple>,
}

/// Keeps everything in a mutex-guarded map, with the same semantics as the
/// SQL adapters.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

fn live(valid_until: Option<i64>, now_ms: i64) -> bool {
    valid_until.is_none_or(|at| at > now_ms)
}

fn live_rows(
    space: &Space,
    p: Id,
    now_ms: i64,
) -> impl Iterator<Item = (Id, Relation)> + '_ {
    space
        .effective
        .range(
            (p, Id(0), Relation::Viewer)..=(p, Id(u128::MAX), Relation::Owner),
        )
        .filter(move |(_, (_, until))| live(*until, now_ms))
        .map(|((_, o, r), _)| (*o, *r))
}

fn poisoned<T>(_: PoisonError<T>) -> StoreError {
    StoreError::Backend("the in-memory store's lock was poisoned".into())
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, StoreError> {
        self.state.lock().map_err(poisoned)
    }

    /// Creates a space or replaces its tree, as a reindex does. Tuples
    /// and effective rows are untouched; call
    /// [`crate::Authz::tree_changed`] afterwards.
    ///
    /// # Errors
    ///
    /// Fails only if the lock is poisoned.
    pub fn set_tree(
        &self,
        space: Id,
        parents: BTreeMap<Id, Option<Id>>,
    ) -> Result<(), StoreError> {
        self.lock()?.spaces.entry(space).or_default().parents = parents;
        Ok(())
    }

    /// The number of effective rows in a space, for tests and metrics.
    ///
    /// # Errors
    ///
    /// Fails if the space is unknown or the lock is poisoned.
    pub fn row_count(&self, space: Id) -> Result<usize, StoreError> {
        let state = self.lock()?;
        let s = state
            .spaces
            .get(&space)
            .ok_or(StoreError::UnknownSpace(space))?;
        Ok(s.effective.len())
    }
}

impl AuthzStore for MemoryStore {
    async fn load_space(&self, space: Id) -> Result<SpaceGraph, StoreError> {
        let state = self.lock()?;
        let s = state
            .spaces
            .get(&space)
            .ok_or(StoreError::UnknownSpace(space))?;
        Ok(SpaceGraph {
            space,
            epoch: s.epoch,
            parents: s.parents.clone(),
            tuples: s.tuples.values().copied().collect(),
        })
    }

    async fn load_directory(&self) -> Result<Vec<Tuple>, StoreError> {
        Ok(self.lock()?.directory.values().copied().collect())
    }

    async fn write_directory(
        &self,
        upsert: Vec<Tuple>,
        delete: Vec<TupleKey>,
    ) -> Result<(), StoreError> {
        let mut state = self.lock()?;
        for key in delete {
            let _ = state.directory.remove(&key);
        }
        for t in upsert {
            let _ = state.directory.insert(t.key(), t);
        }
        Ok(())
    }

    async fn spaces_granting(
        &self,
        groups: &BTreeSet<Id>,
    ) -> Result<Vec<Id>, StoreError> {
        let state = self.lock()?;
        Ok(state
            .spaces
            .iter()
            .filter(|(_, s)| {
                s.tuples.values().any(|t| {
                    matches!(
                        t.subject,
                        Subject::Group(g) if groups.contains(&g)
                    )
                })
            })
            .map(|(id, _)| *id)
            .collect())
    }

    async fn commit(&self, commit: Commit) -> Result<u64, StoreError> {
        let mut state = self.lock()?;
        let s = state
            .spaces
            .get_mut(&commit.space)
            .ok_or(StoreError::UnknownSpace(commit.space))?;
        if s.epoch != commit.expected_epoch {
            return Err(StoreError::Conflict);
        }
        let epoch = s.epoch + 1;
        for key in commit.delete {
            let _ = s.tuples.remove(&key);
        }
        for t in commit.upsert {
            let _ = s.tuples.insert(t.key(), t);
        }
        match commit.scope {
            Scope::All => s.effective.clear(),
            Scope::Objects(objects) => {
                s.effective.retain(|(_, o, _), _| !objects.contains(o));
            }
        }
        for row in commit.rows {
            let _ = s.effective.insert(
                (row.principal.id(), row.object, row.relation),
                (epoch, row.valid_until),
            );
        }
        s.epoch = epoch;
        Ok(epoch)
    }

    async fn lookup(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
        now_ms: i64,
    ) -> Result<Vec<(Relation, u64)>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        Ok(state
            .spaces
            .get(&space)
            .map(|s| {
                s.effective
                    .range(
                        (p, object, Relation::Viewer)
                            ..=(p, object, Relation::Owner),
                    )
                    .filter(|(_, (_, until))| live(*until, now_ms))
                    .map(|((_, _, r), (epoch, _))| (*r, *epoch))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn list(
        &self,
        space: Id,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let state = self.lock()?;
        Ok(state
            .spaces
            .get(&space)
            .map(|s| live_rows(s, principal.id(), now_ms).collect())
            .unwrap_or_default())
    }

    async fn lookup_many(
        &self,
        principal: Principal,
        objects: &[Id],
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        let wanted: BTreeSet<Id> = objects.iter().copied().collect();
        Ok(state
            .spaces
            .values()
            .flat_map(|s| live_rows(s, p, now_ms))
            .filter(|(o, _)| wanted.contains(o))
            .collect())
    }

    async fn spaces_of(
        &self,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<Id>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        Ok(state
            .spaces
            .iter()
            .filter(|(_, s)| live_rows(s, p, now_ms).next().is_some())
            .map(|(id, _)| *id)
            .collect())
    }

    async fn epoch(&self, space: Id) -> Result<u64, StoreError> {
        let state = self.lock()?;
        state
            .spaces
            .get(&space)
            .map(|s| s.epoch)
            .ok_or(StoreError::UnknownSpace(space))
    }
}
