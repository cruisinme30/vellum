// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What the sync service needs from the rest of the system.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use vellum_kernel::{Error, ObjectId, RequestId, SpaceId, UserId};

use crate::domain::{Change, ClientId, Record};

/// A boxed, sendable future, for object-safe traits.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the store remembers about one client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientState {
    /// The user the client is bound to.
    pub user: UserId,
    /// The last mutation processed for it.
    pub last_mutation_id: u64,
}

/// Facts about the log as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogInfo {
    /// Changes when the log is rebuilt.
    pub epoch: u64,
    /// Tombstones at or below this position may have been pruned; a cursor
    /// older than it must reset.
    pub horizon: u64,
    /// The last log position written. A cursor past it did not come from
    /// this log (a restored backup, a corrupted device store) and resets.
    pub head: u64,
}

/// Persistence for clients and the change log.
///
/// The log keeps the latest version of each record (keyed by space, collection
/// and key) at the position of its last write, plus tombstones for deletes
/// until pruned.
pub trait SyncStore: Send + Sync + 'static {
    /// The client's state, if it has pushed before.
    fn client(
        &self,
        client: &ClientId,
    ) -> impl Future<Output = Result<Option<ClientState>, Error>> + Send;

    /// Records that every mutation up to `last_mutation_id` is processed.
    /// Binds the client to `user` on first use; never moves backwards.
    fn advance_client(
        &self,
        client: &ClientId,
        user: UserId,
        last_mutation_id: u64,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Writes changes at new log positions, in order, atomically.
    fn append(
        &self,
        changes: Vec<Change>,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Up to `limit` records written after `after`, in log order. With
    /// `live_only`, tombstones are skipped (used to build a full snapshot).
    fn read(
        &self,
        after: u64,
        limit: usize,
        live_only: bool,
    ) -> impl Future<Output = Result<Vec<Record>, Error>> + Send;

    /// The log's epoch and pruning horizon.
    fn info(&self) -> impl Future<Output = Result<LogInfo, Error>> + Send;
}

/// The part of a change that decides who may read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessKey {
    /// The record's space.
    pub space: SpaceId,
    /// The object that narrows access, if any.
    pub object: Option<ObjectId>,
}

/// Read permissions, from the authorization service.
pub trait ReadAccess: Send + Sync + 'static {
    /// The user's permission epoch: it changes whenever anything the user
    /// can read may have changed. A pull under a new epoch resets.
    fn epoch(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<u64, Error>> + Send;

    /// Which of `keys` the user may read, in one batched check.
    fn readable(
        &self,
        user: UserId,
        keys: &[AccessKey],
    ) -> impl Future<Output = Result<HashSet<AccessKey>, Error>> + Send;
}

/// Who and what a mutator runs for.
#[derive(Debug, Clone)]
pub struct MutationContext {
    /// The signed-in person; mutators enforce their own permissions.
    pub user: UserId,
    /// The device that made the change.
    pub client: ClientId,
    /// With `client`, a unique key for this mutation: a mutator that is
    /// not naturally idempotent stores it to skip a replay after a crash.
    pub mutation_id: u64,
    /// The push request, for logs and traces.
    pub request_id: RequestId,
}

/// The server half of a named mutator. The owning service registers it;
/// it changes that service's own state and returns the record changes
/// clients should see.
pub trait ServerMutator: Send + Sync + 'static {
    /// Applies the mutation.
    ///
    /// # Errors
    ///
    /// A retryable error kind (`unavailable`, `timeout`, `rate_limited`)
    /// stops the push so the client retries later; any other error
    /// rejects this mutation and processing continues.
    fn apply<'a>(
        &'a self,
        context: &'a MutationContext,
        args: Value,
    ) -> BoxFuture<'a, Result<Vec<Change>, Error>>;
}
