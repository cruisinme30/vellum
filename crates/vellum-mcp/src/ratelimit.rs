// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-token rate limits, on the shared GCRA limiter from
//! `vellum-security`, optionally held across every replica through a
//! [`SharedLimit`] (ADR 0019, D218).

use std::sync::Arc;
use std::time::{Duration, Instant};

use vellum_security::rate::{Quota, RateLimiter as Gcra};

/// Limits for one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    /// Burst size: calls allowed at once.
    pub burst: u32,
    /// Sustained calls per minute.
    pub per_minute: u32,
}

impl Default for RateLimit {
    /// 120 calls a minute with bursts of 30: enough for an agent loop,
    /// small enough that a runaway client cannot load the server.
    fn default() -> Self {
        Self {
            burst: 30,
            per_minute: 120,
        }
    }
}

impl RateLimit {
    /// The GCRA quota: `burst` calls per `burst` steady-rate intervals.
    fn quota(self) -> Option<Quota> {
        let per_minute = self.per_minute.max(1);
        let burst = self.burst.max(1);
        let period = Duration::from_mins(1).saturating_mul(burst) / per_minute;
        Quota::new(burst, period.max(Duration::from_millis(1)))
    }
}

/// Most tokens tracked at once; idle, drained keys are evicted first.
const MAX_TOKENS: usize = 100_000;

/// A per-minute allowance counted across replicas.
///
/// Each replica still applies its own burst limit first; this one, such
/// as the server's shared rate-limit table, catches a client spreading
/// calls over replicas.
pub trait SharedLimit: Send + Sync {
    /// Counts one call for `key`. Returns how long to wait when the
    /// allowance is used up.
    fn hit(&self, key: &str) -> Option<Duration>;
}

/// Rate limiter keyed by token id.
pub struct RateLimiter {
    local: Option<Gcra<String>>,
    shared: Option<Arc<dyn SharedLimit>>,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("shared", &self.shared.is_some())
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    /// A limiter applying `limit` to every token.
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self {
            local: limit.quota().map(|q| Gcra::new(q, MAX_TOKENS)),
            shared: None,
        }
    }

    /// Also holds every token to `shared`, keyed `mcp:<token id>`.
    #[must_use]
    pub fn with_shared(mut self, shared: Arc<dyn SharedLimit>) -> Self {
        self.shared = Some(shared);
        self
    }

    /// Takes one call from the token's allowance.
    ///
    /// # Errors
    ///
    /// Returns how long to wait before the next call is allowed.
    pub fn check(&self, token_id: &str) -> Result<(), Duration> {
        self.check_at(token_id, Instant::now())
    }

    /// [`Self::check`] at a given instant.
    ///
    /// # Errors
    ///
    /// Returns how long to wait before the next call is allowed.
    pub fn check_at(&self, token_id: &str, now: Instant) -> Result<(), Duration> {
        // The quota is always valid (both parts are clamped above zero);
        // if it were not, fail closed.
        self.local
            .as_ref()
            .map_or(Err(Duration::from_mins(1)), |g| {
                g.check(&token_id.to_owned(), now)
                    .map_err(|l| l.retry_after)
            })?;
        self.shared
            .as_ref()
            .and_then(|s| s.hit(&format!("mcp:{token_id}")))
            .map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_then_refills_per_token() {
        let limiter = RateLimiter::new(RateLimit {
            burst: 2,
            per_minute: 60,
        });
        let t0 = Instant::now();
        assert!(limiter.check_at("a", t0).is_ok());
        assert!(limiter.check_at("a", t0).is_ok());
        let wait = limiter.check_at("a", t0).err();
        assert!(wait.is_some_and(|w| w <= Duration::from_secs(1)));
        assert!(limiter.check_at("b", t0).is_ok(), "tokens are independent");
        assert!(limiter.check_at("a", t0 + Duration::from_secs(1)).is_ok());
    }

    /// Counts calls per key and refuses past `max`, like the replicas'
    /// shared table.
    #[derive(Default)]
    struct Table {
        max: u64,
        seen: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    }

    impl SharedLimit for Table {
        fn hit(&self, key: &str) -> Option<Duration> {
            let mut seen = self.seen.lock().unwrap();
            let n = seen.entry(key.to_owned()).or_default();
            *n += 1;
            let over = *n > self.max;
            drop(seen);
            over.then_some(Duration::from_secs(30))
        }
    }

    #[test]
    fn the_shared_allowance_holds_across_replicas() {
        let table = Arc::new(Table {
            max: 3,
            ..Table::default()
        });
        let limit = RateLimit {
            burst: 10,
            per_minute: 600,
        };
        // Two replicas, each with its own burst allowance, one table.
        let a = RateLimiter::new(limit).with_shared(Arc::clone(&table) as Arc<dyn SharedLimit>);
        let b = RateLimiter::new(limit).with_shared(Arc::clone(&table) as Arc<dyn SharedLimit>);
        let t0 = Instant::now();
        assert!(a.check_at("tok", t0).is_ok());
        assert!(b.check_at("tok", t0).is_ok());
        assert!(a.check_at("tok", t0).is_ok());
        assert_eq!(b.check_at("tok", t0), Err(Duration::from_secs(30)));
        assert!(a.check_at("other", t0).is_ok(), "tokens are independent");
        assert!(table.seen.lock().unwrap().contains_key("mcp:tok"));
    }
}
