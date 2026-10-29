// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rate limiter and lockout behaviour.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use proptest::prelude::*;
use vellum_security::rate::{Lockout, Quota, RateLimiter};

const SEC: Duration = Duration::from_secs(1);

#[test]
fn burst_then_steady_rate() {
    let limiter = RateLimiter::new(Quota::new(5, 5 * SEC).unwrap(), 100);
    let t0 = Instant::now();
    for _ in 0..5 {
        limiter.check(&"tok", t0).unwrap();
    }
    let err = limiter.check(&"tok", t0).unwrap_err();
    assert_eq!(err.retry_after, SEC);
    assert!(limiter.check(&"other", t0).is_ok(), "keys are independent");
    limiter.check(&"tok", t0 + SEC).unwrap();
    assert!(limiter.check(&"tok", t0 + SEC).is_err());
}

#[test]
fn zero_quotas_are_rejected() {
    assert!(Quota::new(0, SEC).is_none());
    assert!(Quota::new(1, Duration::ZERO).is_none());
}

#[test]
fn key_table_stays_bounded() {
    let limiter = RateLimiter::new(Quota::per_minute(10).unwrap(), 50);
    let t0 = Instant::now();
    for key in 0..10_000_u32 {
        limiter.check(&key, t0).unwrap();
        assert!(limiter.len() <= 50);
    }
}

proptest! {
    #[test]
    fn never_more_than_burst_plus_refill(burst in 1_u32..20, period_ms in 10_u64..10_000, gaps in proptest::collection::vec(0_u64..500, 1..200)) {
        let period = Duration::from_millis(period_ms);
        let limiter = RateLimiter::new(Quota::new(burst, period).unwrap(), 10);
        let t0 = Instant::now();
        let mut now = t0;
        let mut allowed = 0_u32;
        for gap in gaps {
            now += Duration::from_millis(gap);
            if limiter.check(&(), now).is_ok() {
                allowed += 1;
            }
        }
        let elapsed = (now - t0).as_secs_f64();
        let bound = f64::from(burst) + elapsed * f64::from(burst) / period.as_secs_f64() + 1.0;
        prop_assert!(f64::from(allowed) <= bound, "allowed {allowed} > bound {bound}");
    }
}

#[test]
fn lockout_backs_off_exponentially() {
    let lockout = Lockout::new(3, SEC, 60 * SEC);
    let t0 = Instant::now();
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), Duration::ZERO);
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), Duration::ZERO);
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), Duration::ZERO);
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), SEC);
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), 2 * SEC);
    assert!(lockout.check(&"ana@northstar.dev", t0).is_err());
    assert!(lockout.check(&"ana@northstar.dev", t0 + 2 * SEC).is_ok());
    for _ in 0..40 {
        let _ = lockout.fail(&"ana@northstar.dev", t0);
    }
    assert_eq!(lockout.fail(&"ana@northstar.dev", t0), 60 * SEC);
    lockout.succeed(&"ana@northstar.dev");
    assert!(lockout.check(&"ana@northstar.dev", t0).is_ok());
}
