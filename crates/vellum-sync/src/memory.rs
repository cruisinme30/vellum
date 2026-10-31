// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An in-memory [`SyncStore`]: the reference implementation the
//! conformance suite pins down, and the store for tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use vellum_kernel::{Error, ErrorKind, SpaceId, UserId};

use crate::domain::{Change, ClientId, Record};
use crate::ports::{ClientState, LogInfo, SyncStore};

type RecordKey = (SpaceId, String, String);

#[derive(Debug, Default)]
struct State {
    clients: HashMap<ClientId, ClientState>,
    log: BTreeMap<u64, Change>,
    positions: HashMap<RecordKey, u64>,
    head: u64,
    epoch: u64,
    horizon: u64,
}

/// The in-memory store.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<State>,
}

impl MemoryStore {
    /// An empty store at epoch 1.
    #[must_use]
    pub fn new() -> Self {
        Self::with_epoch(1)
    }

    /// An empty store at the given epoch, as after a rebuild.
    #[must_use]
    pub fn with_epoch(epoch: u64) -> Self {
        Self {
            state: Mutex::new(State {
                epoch,
                ..State::default()
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave the maps half-written:
        // every method updates them with infallible operations.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops tombstones at or below `seq` and raises the horizon to it.
    pub fn prune_tombstones(&self, seq: u64) {
        let mut state = self.lock();
        let dead: Vec<u64> = state
            .log
            .range(..=seq)
            .filter(|(_, change)| change.value.is_none())
            .map(|(position, _)| *position)
            .collect();
        for position in dead {
            if let Some(change) = state.log.remove(&position) {
                let _ = state.positions.remove(&(
                    change.space,
                    change.collection,
                    change.key,
                ));
            }
        }
        state.horizon = state.horizon.max(seq);
    }
}

impl SyncStore for MemoryStore {
    async fn client(
        &self,
        client: &ClientId,
    ) -> Result<Option<ClientState>, Error> {
        Ok(self.lock().clients.get(client).copied())
    }

    async fn advance_client(
        &self,
        client: &ClientId,
        user: UserId,
        last_mutation_id: u64,
    ) -> Result<(), Error> {
        let mut state = self.lock();
        let entry =
            state.clients.entry(client.clone()).or_insert(ClientState {
                user,
                last_mutation_id: 0,
            });
        let bound = entry.user == user;
        if bound {
            entry.last_mutation_id =
                entry.last_mutation_id.max(last_mutation_id);
        }
        drop(state);
        if bound {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "this client belongs to another account",
            ))
        }
    }

    async fn append(&self, changes: Vec<Change>) -> Result<(), Error> {
        let mut state = self.lock();
        for change in changes {
            state.head += 1;
            let seq = state.head;
            let key =
                (change.space, change.collection.clone(), change.key.clone());
            if let Some(previous) = state.positions.insert(key, seq) {
                let _ = state.log.remove(&previous);
            }
            let _ = state.log.insert(seq, change);
        }
        drop(state);
        Ok(())
    }

    async fn read(
        &self,
        after: u64,
        limit: usize,
        live_only: bool,
    ) -> Result<Vec<Record>, Error> {
        let state = self.lock();
        Ok(state
            .log
            .range(after.saturating_add(1)..)
            .filter(|(_, change)| !live_only || change.value.is_some())
            .take(limit)
            .map(|(seq, change)| Record {
                change: change.clone(),
                seq: *seq,
            })
            .collect())
    }

    async fn info(&self) -> Result<LogInfo, Error> {
        let state = self.lock();
        Ok(LogInfo {
            epoch: state.epoch,
            horizon: state.horizon,
            head: state.head,
        })
    }
}
