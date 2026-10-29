// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The storage port: what the engine needs from the index.
//!
//! Adapters map it onto the `grants`, `page_ancestors`, `effective_access`
//! and `spaces.authz_epoch` columns of `vellum-db`. [`crate::MemoryStore`]
//! is the reference adapter used by the tests.

use std::collections::BTreeSet;
use std::future::Future;

use crate::graph::SpaceGraph;
use crate::model::{Id, Object, Principal, Relation, Subject, Tuple, TupleRelation};

/// The primary key of a tuple: everything but its expiry.
pub type TupleKey = (Object, TupleRelation, Subject);

/// One row of `effective_access`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectiveRow {
    /// The space object or a node.
    pub object: Id,
    /// Who has access.
    pub principal: Principal,
    /// Their strongest relation.
    pub relation: Relation,
}

/// Which effective rows a commit replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Rows whose object is in the set; rows for other objects stay.
    Objects(BTreeSet<Id>),
    /// Every row of the space.
    All,
}

/// One atomic change to a space: tuple writes, the effective rows they
/// produce, and an epoch bump. Adapters apply it in a single transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The space.
    pub space: Id,
    /// The epoch the rows were computed from. If the space has moved on,
    /// the adapter applies nothing and returns [`StoreError::Conflict`].
    pub expected_epoch: u64,
    /// Tuples to insert or replace (by key).
    pub upsert: Vec<Tuple>,
    /// Tuples to delete.
    pub delete: Vec<TupleKey>,
    /// Which existing rows `rows` replaces.
    pub scope: Scope,
    /// The new rows, stamped with `expected_epoch + 1` by the adapter.
    pub rows: Vec<EffectiveRow>,
}

/// A failure inside a storage adapter.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Another writer bumped the epoch first; reload and retry.
    #[error("the space's authz epoch moved on")]
    Conflict,
    /// The space is not in the index.
    #[error("space {0} is not in the index")]
    UnknownSpace(Id),
    /// Anything else the backend reports.
    #[error(transparent)]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

/// What the engine reads from and writes to the index.
///
/// Every method is one round trip; `lookup` must be a single indexed read
/// (`effective_access` primary key prefix), because it runs on every
/// request.
pub trait AuthzStore: Send + Sync {
    /// The space's tree, tuples and current epoch.
    fn load_space(&self, space: Id) -> impl Future<Output = Result<SpaceGraph, StoreError>> + Send;

    /// Every group membership tuple (they live under [`Id::DIRECTORY`]).
    fn load_directory(&self) -> impl Future<Output = Result<Vec<Tuple>, StoreError>> + Send;

    /// Inserts and deletes membership tuples.
    fn write_directory(
        &self,
        upsert: Vec<Tuple>,
        delete: Vec<TupleKey>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Spaces holding a grant whose subject is one of `groups`.
    fn spaces_granting(
        &self,
        groups: &BTreeSet<Id>,
    ) -> impl Future<Output = Result<Vec<Id>, StoreError>> + Send;

    /// Applies a commit atomically and returns the new epoch.
    fn commit(&self, commit: Commit) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// The principal's relation on one object, with the epoch it was
    /// computed at. One indexed read.
    fn lookup(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
    ) -> impl Future<Output = Result<Option<(Relation, u64)>, StoreError>> + Send;

    /// Every object in the space the principal can reach, with their
    /// relation. One indexed range read; used to filter search and lists.
    fn list(
        &self,
        space: Id,
        principal: Principal,
    ) -> impl Future<Output = Result<Vec<(Id, Relation)>, StoreError>> + Send;

    /// The principal's relation on each of `objects` that it can reach,
    /// across spaces. One indexed read on `(subject_id, object_id)`; used
    /// by search to filter a page of hits.
    fn lookup_many(
        &self,
        principal: Principal,
        objects: &[Id],
    ) -> impl Future<Output = Result<Vec<(Id, Relation)>, StoreError>> + Send;

    /// Every space in which the principal can reach at least one object.
    fn spaces_of(
        &self,
        principal: Principal,
    ) -> impl Future<Output = Result<Vec<Id>, StoreError>> + Send;

    /// The space's current epoch.
    fn epoch(&self, space: Id) -> impl Future<Output = Result<u64, StoreError>> + Send;
}
