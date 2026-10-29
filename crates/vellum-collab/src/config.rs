// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Room tuning. The defaults are the contract from the brief.

use std::time::Duration;

/// How rooms batch, version and bound their work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabConfig {
    /// Longest time an applied update waits before it is appended to the
    /// log. The brief fixes this at no more than 50 ms.
    pub batch_window: Duration,
    /// Quiet time after the last edit before a Markdown version is written.
    pub idle_version_after: Duration,
    /// How long a room with no peers stays open before it closes.
    pub close_after_empty: Duration,
    /// Largest accepted WebSocket frame, in bytes.
    pub max_message_bytes: usize,
    /// Frames queued per peer before it is dropped as a slow consumer.
    pub peer_queue: usize,
    /// Largest awareness state (cursor, name, colour) per client, in bytes.
    pub max_awareness_bytes: usize,
    /// Awareness client IDs one connection may claim.
    pub max_clients_per_peer: usize,
    /// Attempts for one storage write before it is carried into the next.
    pub persist_attempts: u32,
    /// How often an open connection's access is checked again, so a
    /// revoked grant, a trashed page or a signed-out session ends it.
    pub recheck_access_every: Duration,
}

impl Default for CollabConfig {
    fn default() -> Self {
        Self {
            batch_window: Duration::from_millis(50),
            idle_version_after: Duration::from_secs(30),
            close_after_empty: Duration::from_secs(10),
            max_message_bytes: 8 * 1024 * 1024,
            peer_queue: 1024,
            max_awareness_bytes: 16 * 1024,
            max_clients_per_peer: 8,
            persist_attempts: 5,
            recheck_access_every: Duration::from_mins(1),
        }
    }
}
