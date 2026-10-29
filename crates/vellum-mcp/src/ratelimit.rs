// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-token rate limits: one token bucket per access token.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

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

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

/// Token buckets keyed by token id.
#[derive(Debug)]
pub struct RateLimiter {
    limit: RateLimit,
    buckets: Mutex<HashMap<String, Bucket>>,
}

/// Buckets kept before idle, full ones are dropped.
const MAX_BUCKETS: usize = 10_000;

impl RateLimiter {
    /// A limiter applying `limit` to every token.
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self {
            limit,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn refill_per_sec(&self) -> f64 {
        f64::from(self.limit.per_minute) / 60.0
    }

    /// Takes one call from the token's bucket.
    ///
    /// # Errors
    ///
    /// Returns how long to wait before the next call is allowed.
    pub fn check(&self, token_id: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let burst = f64::from(self.limit.burst);
        let rate = self.refill_per_sec();
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() >= MAX_BUCKETS {
            buckets.retain(|_, b| {
                now.duration_since(b.updated)
                    .as_secs_f64()
                    .mul_add(rate, b.tokens)
                    < burst
            });
        }
        let bucket = buckets.entry(token_id.to_owned()).or_insert(Bucket {
            tokens: burst,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = elapsed.mul_add(rate, bucket.tokens).min(burst);
        bucket.updated = now;
        let allowed = bucket.tokens >= 1.0;
        if allowed {
            bucket.tokens -= 1.0;
        }
        let missing = 1.0 - bucket.tokens;
        drop(buckets);
        if allowed {
            Ok(())
        } else if rate > 0.0 {
            Err(Duration::from_secs_f64(missing / rate))
        } else {
            Err(Duration::from_mins(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn bursts_then_refills() {
        let limiter = RateLimiter::new(RateLimit {
            burst: 2,
            per_minute: 60,
        });
        assert!(limiter.check("a").is_ok());
        assert!(limiter.check("a").is_ok());
        let wait = limiter.check("a").err();
        assert!(wait.is_some_and(|w| w <= Duration::from_secs(1)));
        assert!(limiter.check("b").is_ok(), "tokens are independent");
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(limiter.check("a").is_ok());
    }
}
