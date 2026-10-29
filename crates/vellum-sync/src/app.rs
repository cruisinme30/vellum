// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sync use cases: push, pull and publish.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::{Instrument, info, info_span, warn};
use vellum_kernel::{Error, ErrorKind, RequestId, UserId};

use crate::cursor::Cursor;
use crate::domain::{
    Change, ClientId, PullRequest, PullResponse, PushRequest, PushResponse, Record, Rejected,
};
use crate::ports::{AccessKey, MutationContext, ReadAccess, ServerMutator, SyncStore};

/// Most mutations accepted in one push.
pub const MAX_PUSH: usize = 500;
/// Most patches returned by one pull.
pub const PULL_PAGE: usize = 1000;
/// Highest mutation ID: stores keep it as a signed 64-bit number.
pub const MAX_MUTATION_ID: u64 = i64::MAX.unsigned_abs();

/// Mutators by name. Services register theirs at start-up.
#[derive(Default, Clone)]
pub struct Mutators {
    by_name: BTreeMap<String, Arc<dyn ServerMutator>>,
}

impl Mutators {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a mutator under `name` (`<service>.<verb>`, such as
    /// `work.setState`). The name must match the client's.
    ///
    /// # Errors
    ///
    /// `Conflict` when the name is taken: two services must never answer
    /// the same mutation.
    pub fn register(&mut self, name: &str, mutator: Arc<dyn ServerMutator>) -> Result<(), Error> {
        if self.by_name.contains_key(name) {
            return Err(Error::new(
                ErrorKind::Conflict,
                format!("mutator {name} is already registered"),
            ));
        }
        let _ = self.by_name.insert(name.to_owned(), mutator);
        Ok(())
    }

    /// Registered names, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.by_name.keys().map(String::as_str).collect()
    }
}

impl std::fmt::Debug for Mutators {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mutators")
            .field("names", &self.names())
            .finish()
    }
}

/// The sync engine over a store and the read-permission port.
#[derive(Debug)]
pub struct Sync<S, A> {
    store: S,
    access: A,
    mutators: Mutators,
    /// Clients with a push in progress on this node.
    pushing: Mutex<BTreeSet<ClientId>>,
}

/// Marks a client's push as running until dropped, even when the push
/// future is cancelled because the device hung up.
struct Pushing<'a> {
    pushing: &'a Mutex<BTreeSet<ClientId>>,
    client: ClientId,
}

impl<'a> Pushing<'a> {
    /// Claims the client, or refuses with a retryable error while another
    /// push for it runs. Running both would apply its mutations twice:
    /// each would read the same last processed ID before either advanced it.
    fn claim(pushing: &'a Mutex<BTreeSet<ClientId>>, client: &ClientId) -> Result<Self, Error> {
        let claimed = pushing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(client.clone());
        if claimed {
            Ok(Self {
                pushing,
                client: client.clone(),
            })
        } else {
            Err(Error::new(
                ErrorKind::RateLimited,
                "this device's previous push is still running; retry shortly",
            ))
        }
    }
}

impl Drop for Pushing<'_> {
    fn drop(&mut self) {
        let _ = self
            .pushing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.client);
    }
}

/// How one mutation ended.
enum Outcome {
    Applied,
    Rejected(Rejected),
    Retry(Error),
}

impl<S: SyncStore, A: ReadAccess> Sync<S, A> {
    /// Builds the engine.
    pub const fn new(store: S, access: A, mutators: Mutators) -> Self {
        Self {
            store,
            access,
            mutators,
            pushing: Mutex::new(BTreeSet::new()),
        }
    }

    /// The store, for adapters that share it.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Records changes made outside a push (REST, GraphQL, MCP, file
    /// watcher) so clients pull them too.
    ///
    /// # Errors
    ///
    /// Store failures.
    pub async fn publish(&self, changes: Vec<Change>) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        let count = changes.len();
        self.store.append(changes).await?;
        info!(monotonic_counter.vellum.sync.published = count as u64);
        Ok(())
    }

    /// Applies a client's pending mutations, each at most once and in order.
    ///
    /// # Errors
    ///
    /// `InvalidInput` for an oversized or out-of-order batch,
    /// `RateLimited` while another push for the same client runs,
    /// `PermissionDenied` when the client belongs to another user, and
    /// store failures. A retryable mutator failure is not an error: the
    /// response simply stops before that mutation.
    pub async fn push(
        &self,
        user: UserId,
        request_id: RequestId,
        request: PushRequest,
    ) -> Result<PushResponse, Error> {
        let span = info_span!("sync.push", client = %request.client_id, mutations = request.mutations.len());
        async move {
            if request.mutations.len() > MAX_PUSH {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("at most {MAX_PUSH} mutations per push"),
                ));
            }
            if request
                .mutations
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a.id >= b.id))
            {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "mutation ids must increase",
                ));
            }
            if request
                .mutations
                .last()
                .is_some_and(|m| m.id > MAX_MUTATION_ID)
            {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("mutation ids must be at most {MAX_MUTATION_ID}"),
                ));
            }
            let _running = Pushing::claim(&self.pushing, &request.client_id)?;
            let already = self.bound_client(user, &request).await?;
            let mut last = already;
            let mut rejected = Vec::new();
            for mutation in request.mutations.into_iter().filter(|m| m.id > already) {
                let context = MutationContext {
                    user,
                    client: request.client_id.clone(),
                    mutation_id: mutation.id,
                    request_id,
                };
                match self.run(&context, &mutation.name, mutation.args).await? {
                    Outcome::Applied => {}
                    Outcome::Rejected(entry) => rejected.push(entry),
                    Outcome::Retry(error) => {
                        warn!(
                            mutation = mutation.id,
                            code = error.kind().code(),
                            "mutation will be retried"
                        );
                        break;
                    }
                }
                self.store
                    .advance_client(&request.client_id, user, mutation.id)
                    .await?;
                last = mutation.id;
            }
            Ok(PushResponse {
                last_mutation_id: last,
                rejected,
            })
        }
        .instrument(span)
        .await
    }

    /// The client's last processed mutation, after checking it is this user's.
    async fn bound_client(&self, user: UserId, request: &PushRequest) -> Result<u64, Error> {
        match self.store.client(&request.client_id).await? {
            Some(state) if state.user != user => Err(foreign_client()),
            Some(state) => Ok(state.last_mutation_id),
            None => Ok(0),
        }
    }

    async fn run(
        &self,
        context: &MutationContext,
        name: &str,
        args: serde_json::Value,
    ) -> Result<Outcome, Error> {
        let Some(mutator) = self.mutators.by_name.get(name) else {
            info!(
                monotonic_counter.vellum.sync.mutations = 1_u64,
                outcome = "unknown"
            );
            return Ok(Outcome::Rejected(Rejected {
                id: context.mutation_id,
                code: ErrorKind::InvalidInput.code().to_owned(),
                message: format!("unknown mutation {name}"),
            }));
        };
        match mutator.apply(context, args).await {
            Ok(changes) => {
                self.store.append(changes).await?;
                info!(
                    monotonic_counter.vellum.sync.mutations = 1_u64,
                    outcome = "applied"
                );
                Ok(Outcome::Applied)
            }
            Err(error) if error.kind().is_retryable() => {
                info!(
                    monotonic_counter.vellum.sync.mutations = 1_u64,
                    outcome = "retry"
                );
                Ok(Outcome::Retry(error))
            }
            Err(error) => {
                info!(
                    monotonic_counter.vellum.sync.mutations = 1_u64,
                    outcome = "rejected"
                );
                Ok(Outcome::Rejected(Rejected {
                    id: context.mutation_id,
                    code: error.kind().code().to_owned(),
                    message: error.message().to_owned(),
                }))
            }
        }
    }

    /// Returns the changes since the client's cursor that the user may read.
    ///
    /// # Errors
    ///
    /// `PermissionDenied` when the client belongs to another user, and
    /// store or permission-check failures.
    pub async fn pull(&self, user: UserId, request: PullRequest) -> Result<PullResponse, Error> {
        let span =
            info_span!("sync.pull", client = %request.client_id, reset = tracing::field::Empty);
        async move {
            let last_mutation_id = match self.store.client(&request.client_id).await? {
                Some(state) if state.user != user => return Err(foreign_client()),
                Some(state) => state.last_mutation_id,
                None => 0,
            };
            let info = self.store.info().await?;
            let access_epoch = self.access.epoch(user).await?;
            let resume = Cursor::parse(&request.cursor).filter(|cursor| {
                cursor.log_epoch == info.epoch
                    && cursor.access_epoch == access_epoch
                    && cursor.seq >= info.horizon
                    && cursor.seq <= info.head
            });
            let reset = resume.is_none();
            let _ = tracing::Span::current().record("reset", reset);
            let after = resume.map_or(0, |cursor| cursor.seq);
            let mut records = self.store.read(after, PULL_PAGE + 1, reset).await?;
            let more = records.len() > PULL_PAGE;
            records.truncate(PULL_PAGE);
            let seq = records.last().map_or(after, |record| record.seq);
            let patches = self.visible(user, &records).await?;
            info!(
                histogram.vellum.sync.pull_patches = patches.len() as u64,
                reset
            );
            Ok(PullResponse {
                cursor: Cursor {
                    log_epoch: info.epoch,
                    access_epoch,
                    seq,
                }
                .to_string(),
                last_mutation_id,
                reset,
                patches,
                more,
            })
        }
        .instrument(span)
        .await
    }

    async fn visible(
        &self,
        user: UserId,
        records: &[Record],
    ) -> Result<Vec<crate::domain::Patch>, Error> {
        let keys: Vec<AccessKey> = records
            .iter()
            .map(|record| AccessKey {
                space: record.change.space,
                object: record.change.object,
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let readable = if keys.is_empty() {
            HashSet::new()
        } else {
            self.access.readable(user, &keys).await?
        };
        Ok(records
            .iter()
            .filter(|record| {
                readable.contains(&AccessKey {
                    space: record.change.space,
                    object: record.change.object,
                })
            })
            .map(|record| record.change.to_patch())
            .collect())
    }
}

fn foreign_client() -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        "this client belongs to another account",
    )
}
