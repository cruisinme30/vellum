// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! In-memory adapters for the [`crate::ports`], for tests and for trying
//! the service without a database.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use vellum_kernel::{Error, ErrorKind, Result};

use crate::hub::RoomKey;
use crate::ports::{
    DocumentFiles, MarkdownVersion, Seq, StoredUpdate, UpdateStore,
};

/// An [`UpdateStore`] in a map. It can be told to fail, to exercise retries.
#[derive(Debug, Default)]
pub struct MemoryUpdateStore {
    logs: Mutex<HashMap<RoomKey, Log>>,
    failing: AtomicBool,
}

#[derive(Debug, Default)]
struct Log {
    next_seq: Seq,
    records: Vec<StoredUpdate>,
}

impl MemoryUpdateStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// While `true`, every call fails with [`ErrorKind::Unavailable`].
    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    /// How many records the room's log holds.
    #[must_use]
    pub fn record_count(&self, room: RoomKey) -> usize {
        self.logs().get(&room).map_or(0, |log| log.records.len())
    }

    fn logs(&self) -> MutexGuard<'_, HashMap<RoomKey, Log>> {
        self.logs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn check(&self) -> Result<()> {
        if self.failing.load(Ordering::SeqCst) {
            Err(Error::new(ErrorKind::Unavailable, "update store offline"))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl UpdateStore for MemoryUpdateStore {
    async fn load(&self, room: RoomKey) -> Result<Vec<StoredUpdate>> {
        self.check()?;
        Ok(self
            .logs()
            .get(&room)
            .map(|log| log.records.clone())
            .unwrap_or_default())
    }

    async fn append(&self, room: RoomKey, update: Vec<u8>) -> Result<Seq> {
        self.check()?;
        let mut logs = self.logs();
        let log = logs.entry(room).or_default();
        log.next_seq += 1;
        let seq = log.next_seq;
        log.records.push(StoredUpdate { seq, update });
        drop(logs);
        Ok(seq)
    }

    async fn seed(&self, room: RoomKey, update: Vec<u8>) -> Result<bool> {
        self.check()?;
        let mut logs = self.logs();
        let log = logs.entry(room).or_default();
        let empty = log.records.is_empty();
        if empty {
            log.next_seq += 1;
            let seq = log.next_seq;
            log.records.push(StoredUpdate { seq, update });
        }
        drop(logs);
        Ok(empty)
    }

    async fn compact(
        &self,
        room: RoomKey,
        through: Seq,
        merged: Vec<u8>,
    ) -> Result<()> {
        self.check()?;
        let mut logs = self.logs();
        let log = logs.entry(room).or_default();
        log.records.retain(|record| record.seq > through);
        log.records.insert(
            0,
            StoredUpdate {
                seq: through,
                update: merged,
            },
        );
        drop(logs);
        Ok(())
    }
}

/// A [`DocumentFiles`] in a map, keeping every version written.
#[derive(Debug, Default)]
pub struct MemoryFiles {
    files: Mutex<HashMap<RoomKey, Vec<MarkdownVersion>>>,
    initial: Mutex<HashMap<RoomKey, String>>,
}

impl MemoryFiles {
    /// No files.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts a file in place before any room opens it.
    pub fn put(&self, room: RoomKey, markdown: impl Into<String>) {
        let _previous = self
            .initial
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(room, markdown.into());
    }

    /// Every version written for the room, oldest first.
    #[must_use]
    pub fn versions(&self, room: RoomKey) -> Vec<MarkdownVersion> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&room)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl DocumentFiles for MemoryFiles {
    async fn read_markdown(&self, room: RoomKey) -> Result<Option<String>> {
        if let Some(latest) = self.versions(room).pop() {
            return Ok(Some(latest.markdown));
        }
        Ok(self
            .initial
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&room)
            .cloned())
    }

    async fn write_version(
        &self,
        room: RoomKey,
        version: MarkdownVersion,
    ) -> Result<()> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(room)
            .or_default()
            .push(version);
        Ok(())
    }
}
