// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What the Work service needs from the rest of the system.
//!
//! The binary wires real adapters (identity for [`Directory`], the event
//! log for [`EventSink`]); tests use the simple ones here.

use std::collections::HashMap;
use std::sync::Mutex;

use time::OffsetDateTime;
use vellum_contracts::BoxFuture;
use vellum_kernel::UserId;
use vellum_work::{UserRef, WorkEvent};

/// Maps signed-in users to the handles Work files use (`dev`, `priya`).
pub trait Directory: Send + Sync + 'static {
    /// The handle of `user`, or `None` if the account has none.
    fn handle(&self, user: UserId) -> BoxFuture<'_, Option<UserRef>>;
}

/// A fixed user list, for tests and single-user installs.
#[derive(Debug, Default)]
pub struct StaticDirectory {
    users: HashMap<UserId, UserRef>,
}

impl StaticDirectory {
    /// An empty directory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a user.
    #[must_use]
    pub fn with(mut self, user: UserId, handle: UserRef) -> Self {
        let _ = self.users.insert(user, handle);
        self
    }
}

impl Directory for StaticDirectory {
    fn handle(&self, user: UserId) -> BoxFuture<'_, Option<UserRef>> {
        let found = self.users.get(&user).cloned();
        Box::pin(async move { found })
    }
}

/// Where Work events go: the event log, which feeds webhooks,
/// notifications, Git automation, search and My Work.
///
/// `publish` is called after the files are written, in order, and must not
/// block; adapters queue.
pub trait EventSink: Send + Sync + 'static {
    /// Takes the events of one command.
    fn publish(&self, events: Vec<WorkEvent>);
}

/// Keeps every event in memory, for tests and the demo.
#[derive(Debug, Default)]
pub struct EventLog {
    events: Mutex<Vec<WorkEvent>>,
}

impl EventLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the events published so far.
    #[must_use]
    pub fn take(&self) -> Vec<WorkEvent> {
        self.events
            .lock()
            .map(|mut e| std::mem::take(&mut *e))
            .unwrap_or_default()
    }
}

impl EventSink for EventLog {
    fn publish(&self, events: Vec<WorkEvent>) {
        if let Ok(mut log) = self.events.lock() {
            log.extend(events);
        }
    }
}

/// The time source, so tests can fix "now".
pub trait Clock: Send + Sync + 'static {
    /// The current time.
    fn now(&self) -> OffsetDateTime;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// A clock stopped at one instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub OffsetDateTime);

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        self.0
    }
}
