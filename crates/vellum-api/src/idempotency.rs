// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Idempotent writes with the `Idempotency-Key` header.
//!
//! A client that may retry a write sends a unique key with it. The first
//! request with a key runs and its response is stored; a retry with the same
//! key and the same request replays the stored response; a concurrent retry
//! gets `409 conflict`; a different request reusing the key gets `422`.
//! Keys are scoped to the caller, so two users cannot collide. Server errors
//! are not stored, so a retry after a 5xx runs again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use http::{HeaderName, StatusCode};
use vellum_contracts::BoxFuture;

/// The request header carrying the key.
pub const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
/// The response header set on a replayed response.
pub const IDEMPOTENT_REPLAYED: HeaderName = HeaderName::from_static("idempotent-replayed");
/// Longest accepted key.
pub const MAX_KEY_LEN: usize = 255;

/// A key, scoped to its caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScopedKey {
    /// The caller: a user ID, or the client IP for anonymous callers.
    pub caller: String,
    /// The client-chosen key.
    pub key: String,
}

/// A stored response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    /// The status code.
    pub status: StatusCode,
    /// The `Content-Type`, if any.
    pub content_type: Option<String>,
    /// The body.
    pub body: Vec<u8>,
}

/// What to do with a request carrying a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begin {
    /// First time: run it, then call [`IdempotencyStore::complete`] or
    /// [`IdempotencyStore::abandon`].
    Proceed,
    /// Seen and finished: replay this.
    Replay(StoredResponse),
    /// Seen and still running.
    InFlight,
    /// Seen with a different request.
    Mismatch,
}

/// Where keys and responses are kept. The single binary keeps them in
/// memory; replicated deployments use the database adapter so a retry can
/// land on another replica.
pub trait IdempotencyStore: Send + Sync + 'static {
    /// Claims `key` for a request whose method, path and body hash to
    /// `fingerprint`.
    fn begin(&self, key: ScopedKey, fingerprint: [u8; 32], ttl: Duration) -> BoxFuture<'_, Begin>;
    /// Stores the response for a claimed key.
    fn complete(&self, key: ScopedKey, response: StoredResponse) -> BoxFuture<'_, ()>;
    /// Releases a claimed key without storing (server error, client gone).
    fn abandon(&self, key: ScopedKey) -> BoxFuture<'_, ()>;
}

#[derive(Debug)]
enum Entry {
    Running {
        fingerprint: [u8; 32],
        expires: Instant,
    },
    Done {
        fingerprint: [u8; 32],
        response: StoredResponse,
        expires: Instant,
    },
}

impl Entry {
    const fn expires(&self) -> Instant {
        match self {
            Self::Running { expires, .. } | Self::Done { expires, .. } => *expires,
        }
    }
}

/// An in-memory store, bounded in entries.
#[derive(Debug)]
pub struct MemoryIdempotencyStore {
    capacity: usize,
    entries: Mutex<HashMap<ScopedKey, Entry>>,
}

impl MemoryIdempotencyStore {
    /// A store holding at most `capacity` keys; expired keys are pruned
    /// first, then new keys are refused storage (they still run).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ScopedKey, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for MemoryIdempotencyStore {
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl IdempotencyStore for MemoryIdempotencyStore {
    fn begin(&self, key: ScopedKey, fingerprint: [u8; 32], ttl: Duration) -> BoxFuture<'_, Begin> {
        let now = Instant::now();
        let mut entries = self.lock();
        if entries.get(&key).is_some_and(|e| e.expires() <= now) {
            let _ = entries.remove(&key);
        }
        let verdict = match entries.get(&key) {
            Some(Entry::Running { fingerprint: f, .. }) if *f == fingerprint => Begin::InFlight,
            Some(Entry::Done {
                fingerprint: f,
                response,
                ..
            }) if *f == fingerprint => Begin::Replay(response.clone()),
            Some(_) => Begin::Mismatch,
            None => {
                if entries.len() >= self.capacity {
                    entries.retain(|_, e| e.expires() > now);
                }
                if entries.len() < self.capacity {
                    let _ = entries.insert(
                        key,
                        Entry::Running {
                            fingerprint,
                            expires: now + ttl,
                        },
                    );
                }
                Begin::Proceed
            }
        };
        drop(entries);
        Box::pin(std::future::ready(verdict))
    }

    fn complete(&self, key: ScopedKey, response: StoredResponse) -> BoxFuture<'_, ()> {
        let mut entries = self.lock();
        if let Some(Entry::Running {
            fingerprint,
            expires,
        }) = entries.get(&key)
        {
            let entry = Entry::Done {
                fingerprint: *fingerprint,
                response,
                expires: *expires,
            };
            let _ = entries.insert(key, entry);
        }
        drop(entries);
        Box::pin(std::future::ready(()))
    }

    fn abandon(&self, key: ScopedKey) -> BoxFuture<'_, ()> {
        let mut entries = self.lock();
        if matches!(entries.get(&key), Some(Entry::Running { .. })) {
            let _ = entries.remove(&key);
        }
        drop(entries);
        Box::pin(std::future::ready(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> ScopedKey {
        ScopedKey {
            caller: "u1".into(),
            key: k.into(),
        }
    }

    #[tokio::test]
    async fn lifecycle() {
        let store = MemoryIdempotencyStore::new(10);
        let ttl = Duration::from_mins(1);
        assert_eq!(store.begin(key("a"), [1; 32], ttl).await, Begin::Proceed);
        assert_eq!(store.begin(key("a"), [1; 32], ttl).await, Begin::InFlight);
        assert_eq!(store.begin(key("a"), [2; 32], ttl).await, Begin::Mismatch);
        let resp = StoredResponse {
            status: StatusCode::CREATED,
            content_type: None,
            body: b"{}".to_vec(),
        };
        store.complete(key("a"), resp.clone()).await;
        assert_eq!(
            store.begin(key("a"), [1; 32], ttl).await,
            Begin::Replay(resp)
        );
        assert_eq!(store.begin(key("b"), [1; 32], ttl).await, Begin::Proceed);
        store.abandon(key("b")).await;
        assert_eq!(store.begin(key("b"), [1; 32], ttl).await, Begin::Proceed);
        let other = ScopedKey {
            caller: "u2".into(),
            key: "a".into(),
        };
        assert_eq!(store.begin(other, [9; 32], ttl).await, Begin::Proceed);
    }

    #[tokio::test]
    async fn expiry_and_capacity() {
        let store = MemoryIdempotencyStore::new(1);
        assert_eq!(
            store.begin(key("a"), [1; 32], Duration::ZERO).await,
            Begin::Proceed
        );
        assert_eq!(
            store.begin(key("a"), [1; 32], Duration::ZERO).await,
            Begin::Proceed,
            "expired"
        );
        assert_eq!(
            store.begin(key("b"), [1; 32], Duration::from_mins(1)).await,
            Begin::Proceed
        );
        assert_eq!(
            store.begin(key("c"), [1; 32], Duration::from_mins(1)).await,
            Begin::Proceed
        );
        assert_eq!(
            store.begin(key("c"), [1; 32], Duration::from_mins(1)).await,
            Begin::Proceed,
            "full: not stored"
        );
    }
}
