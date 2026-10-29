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
    /// `(principal id, object) → (relation, epoch)`, the shape of the
    /// `effective_access` primary key.
    effective: BTreeMap<(Id, Id), (Relation, u64)>,
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
    pub fn set_tree(&self, space: Id, parents: BTreeMap<Id, Option<Id>>) -> Result<(), StoreError> {
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

    async fn spaces_granting(&self, groups: &BTreeSet<Id>) -> Result<Vec<Id>, StoreError> {
        let state = self.lock()?;
        Ok(state
            .spaces
            .iter()
            .filter(|(_, s)| {
                s.tuples
                    .values()
                    .any(|t| matches!(t.subject, Subject::Group(g) if groups.contains(&g)))
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
            Scope::Objects(objects) => s.effective.retain(|(_, o), _| !objects.contains(o)),
        }
        for row in commit.rows {
            let _ = s
                .effective
                .insert((row.principal.id(), row.object), (row.relation, epoch));
        }
        s.epoch = epoch;
        Ok(epoch)
    }

    async fn lookup(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
    ) -> Result<Option<(Relation, u64)>, StoreError> {
        let state = self.lock()?;
        Ok(state
            .spaces
            .get(&space)
            .and_then(|s| s.effective.get(&(principal.id(), object)).copied()))
    }

    async fn list(
        &self,
        space: Id,
        principal: Principal,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        Ok(state
            .spaces
            .get(&space)
            .map(|s| {
                s.effective
                    .range((p, Id(0))..=(p, Id(u128::MAX)))
                    .map(|((_, object), (relation, _))| (*object, *relation))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn lookup_many(
        &self,
        principal: Principal,
        objects: &[Id],
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        Ok(objects
            .iter()
            .filter_map(|o| {
                state
                    .spaces
                    .values()
                    .find_map(|s| s.effective.get(&(p, *o)))
                    .map(|(r, _)| (*o, *r))
            })
            .collect())
    }

    async fn spaces_of(&self, principal: Principal) -> Result<Vec<Id>, StoreError> {
        let state = self.lock()?;
        let p = principal.id();
        Ok(state
            .spaces
            .iter()
            .filter(|(_, s)| {
                s.effective
                    .range((p, Id(0))..=(p, Id(u128::MAX)))
                    .next()
                    .is_some()
            })
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
