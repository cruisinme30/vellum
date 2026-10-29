// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-token rate limits, on the shared GCRA limiter from
//! `vellum-security`.

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

/// Rate limiter keyed by token id.
#[derive(Debug)]
pub struct RateLimiter(Option<Gcra<String>>);

impl RateLimiter {
    /// A limiter applying `limit` to every token.
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self(limit.quota().map(|q| Gcra::new(q, MAX_TOKENS)))
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
        self.0.as_ref().map_or(Err(Duration::from_mins(1)), |g| {
            g.check(&token_id.to_owned(), now)
                .map_err(|l| l.retry_after)
        })
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
}
