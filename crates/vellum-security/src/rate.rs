// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rate limits and sign-in lockout.
//!
//! [`RateLimiter`] implements the generic cell rate algorithm (GCRA): one
//! timestamp per key, no background timers, and a precise `retry_after`
//! for the `Retry-After` header. Keys are whatever the caller limits on:
//! API token, user, IP address, MCP client. The table is bounded; when it
//! is full, keys whose bucket has fully drained are evicted first.
//!
//! [`Lockout`] slows down repeated failures (sign-in, share-link
//! passwords) with an exponential back-off per key.
//!
//! Both take the current [`Instant`] as an argument, so tests and callers
//! control time. In the Organisation profile the same algorithm runs
//! against Redis with this in-process limiter as the fallback.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The address that limits and lockouts count a client under: an `IPv4`
/// address as is, and an `IPv6` address as its `/64` network.
///
/// One `IPv6` customer is usually given a whole `/64`, so keying on the
/// full address let a single client rotate through 2^64 addresses and
/// never reach a per-address limit (red team round 4). `IPv4`-mapped
/// addresses count as the `IPv4` address they carry.
#[must_use]
pub fn client_bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or_else(
            || IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !((1_u128 << 64) - 1))),
            IpAddr::V4,
        ),
    }
}

/// How many requests a key may make: `burst` at once, refilling at one
/// request every `period / burst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    burst: u32,
    period: Duration,
}

impl Quota {
    /// `burst` requests per `period`. Returns `None` if either is zero.
    #[must_use]
    pub fn new(burst: u32, period: Duration) -> Option<Self> {
        (burst > 0 && !period.is_zero()).then_some(Self { burst, period })
    }

    /// Requests per minute, with the whole minute's allowance as burst.
    #[must_use]
    pub fn per_minute(requests: u32) -> Option<Self> {
        Self::new(requests, Duration::from_mins(1))
    }

    /// The spacing between requests at the steady rate.
    fn interval(self) -> Duration {
        self.period / self.burst
    }

    /// How far ahead of `now` the theoretical arrival time may run.
    fn tolerance(self) -> Duration {
        self.period.saturating_sub(self.interval())
    }
}

/// A refused request: try again after this long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rate limited; retry after {retry_after:?}")]
pub struct Limited {
    /// Time until the next request would be allowed.
    pub retry_after: Duration,
}

/// A keyed GCRA rate limiter with a bounded key table.
#[derive(Debug)]
pub struct RateLimiter<K> {
    quota: Quota,
    max_keys: usize,
    // Theoretical arrival time per key.
    table: Mutex<HashMap<K, Instant>>,
}

impl<K: Eq + Hash + Clone> RateLimiter<K> {
    /// Creates a limiter tracking at most `max_keys` keys.
    #[must_use]
    pub fn new(quota: Quota, max_keys: usize) -> Self {
        Self {
            quota,
            max_keys: max_keys.max(1),
            table: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one request for `key` at `now`.
    ///
    /// # Errors
    ///
    /// Returns [`Limited`] when the key has used its quota.
    pub fn check(&self, key: &K, now: Instant) -> Result<(), Limited> {
        let mut table = self
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tat = table.get(key).copied().unwrap_or(now).max(now);
        let allow_at = tat.checked_sub(self.quota.tolerance()).unwrap_or(now);
        if allow_at > now {
            return Err(Limited {
                retry_after: allow_at - now,
            });
        }
        if !table.contains_key(key) && table.len() >= self.max_keys {
            Self::evict(&mut table, now, self.max_keys);
        }
        let _previous = table.insert(key.clone(), tat + self.quota.interval());
        drop(table);
        Ok(())
    }

    /// Drops drained keys; if none are drained, drops the soonest-drained half.
    fn evict(table: &mut HashMap<K, Instant>, now: Instant, max_keys: usize) {
        table.retain(|_, tat| *tat > now);
        if table.len() >= max_keys {
            let mut tats: Vec<Instant> = table.values().copied().collect();
            tats.sort_unstable();
            if let Some(&cutoff) = tats.get(tats.len() / 2) {
                table.retain(|_, tat| *tat > cutoff);
            }
        }
    }

    /// Number of keys currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.table.lock().map_or(0, |t| t.len())
    }

    /// Whether no keys are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Keys a [`Lockout`] tracks unless told otherwise.
pub const DEFAULT_LOCKOUT_KEYS: usize = 100_000;

/// Exponential back-off after repeated failures.
///
/// The first `free_attempts` failures cost nothing; each later failure
/// doubles the wait, starting at `base` and capped at `max`. A success
/// clears the key.
///
/// The table is bounded (#273): a key whose lockout ended more than `max`
/// ago is forgotten, and when the table is full the entries that unlock
/// soonest are dropped first, so a flood of new keys cannot exhaust memory
/// and evicts the attacker's own heavily locked keys last.
#[derive(Debug)]
pub struct Lockout<K> {
    free_attempts: u32,
    base: Duration,
    max: Duration,
    max_keys: usize,
    // Failures so far and when the key may try again.
    state: Mutex<HashMap<K, (u32, Instant)>>,
}

impl<K: Eq + Hash + Clone> Lockout<K> {
    /// Creates a lockout policy.
    #[must_use]
    pub fn new(free_attempts: u32, base: Duration, max: Duration) -> Self {
        Self::with_max_keys(free_attempts, base, max, DEFAULT_LOCKOUT_KEYS)
    }

    /// Creates a lockout policy tracking at most `max_keys` keys.
    #[must_use]
    pub fn with_max_keys(
        free_attempts: u32,
        base: Duration,
        max: Duration,
        max_keys: usize,
    ) -> Self {
        Self {
            free_attempts,
            base,
            max,
            max_keys: max_keys.max(1),
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Number of keys currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().map_or(0, |t| t.len())
    }

    /// Whether no keys are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forgets keys whose lockout ended more than `max` ago; if the table
    /// is still full, drops the half that unlocks soonest.
    fn evict(&self, state: &mut HashMap<K, (u32, Instant)>, now: Instant) {
        state.retain(|_, (_, until)| until.checked_add(self.max).is_none_or(|t| t > now));
        if state.len() >= self.max_keys {
            let mut untils: Vec<Instant> = state.values().map(|(_, until)| *until).collect();
            untils.sort_unstable();
            if let Some(&cutoff) = untils.get(untils.len() / 2) {
                state.retain(|_, (_, until)| *until > cutoff);
            }
        }
    }

    /// Whether `key` may try now.
    ///
    /// # Errors
    ///
    /// Returns [`Limited`] while the key is locked out.
    pub fn check(&self, key: &K, now: Instant) -> Result<(), Limited> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.get(key) {
            Some(&(_, until)) if until > now => Err(Limited {
                retry_after: until - now,
            }),
            _ => Ok(()),
        }
    }

    /// Records a failure and returns how long the key is now locked out.
    pub fn fail(&self, key: &K, now: Instant) -> Duration {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.contains_key(key) && state.len() >= self.max_keys {
            self.evict(&mut state, now);
        }
        let failures = state.get(key).map_or(0, |&(n, _)| n).saturating_add(1);
        let over = failures.saturating_sub(self.free_attempts);
        let wait = if over == 0 {
            Duration::ZERO
        } else {
            let factor = 1_u32.checked_shl(over - 1).unwrap_or(u32::MAX);
            self.base.saturating_mul(factor).min(self.max)
        };
        let _previous = state.insert(key.clone(), (failures, now + wait));
        wait
    }

    /// Clears the key after a success.
    pub fn succeed(&self, key: &K) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _removed = state.remove(key);
    }
}
