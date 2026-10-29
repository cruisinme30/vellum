// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A circuit breaker per service.
//!
//! When a service keeps failing with `unavailable`, `timeout` or `internal`
//! errors, the breaker opens and the gateway fails fast with `unavailable`
//! instead of queueing more work on it. After a cool-down one probe request
//! is let through; success closes the breaker, failure re-opens it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use vellum_kernel::ErrorKind;

/// When a breaker trips and how long it stays open.
#[derive(Debug, Clone, Copy)]
pub struct BreakerPolicy {
    /// Consecutive failures that open the breaker.
    pub failure_threshold: u32,
    /// How long the breaker stays open before a probe.
    pub cool_down: Duration,
}

impl Default for BreakerPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cool_down: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed { failures: u32 },
    Open { until: Instant },
    HalfOpen { probing: bool },
}

/// The externally visible state, for `/readyz` and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Requests flow.
    Closed,
    /// Requests fail fast.
    Open,
    /// One probe request is allowed.
    HalfOpen,
}

/// A circuit breaker.
#[derive(Debug)]
pub struct CircuitBreaker {
    policy: BreakerPolicy,
    state: Mutex<State>,
}

/// Whether an error counts against the service's health. Caller mistakes
/// (`invalid_input`, `not_found`, …) do not.
#[must_use]
pub const fn is_failure(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Unavailable | ErrorKind::Timeout | ErrorKind::Internal
    )
}

impl CircuitBreaker {
    /// A closed breaker.
    #[must_use]
    pub const fn new(policy: BreakerPolicy) -> Self {
        Self {
            policy,
            state: Mutex::new(State::Closed { failures: 0 }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Asks to send a request. `false` means fail fast.
    pub fn try_acquire(&self, now: Instant) -> bool {
        let mut state = self.lock();
        match *state {
            State::Closed { .. } => true,
            State::Open { until } if now >= until => {
                *state = State::HalfOpen { probing: true };
                true
            }
            State::Open { .. } | State::HalfOpen { probing: true } => false,
            State::HalfOpen { probing: false } => {
                *state = State::HalfOpen { probing: true };
                true
            }
        }
    }

    /// Records the outcome of a request that was let through.
    pub fn record(&self, success: bool, now: Instant) {
        let mut state = self.lock();
        *state = match (*state, success) {
            (_, true) => State::Closed { failures: 0 },
            (State::Closed { failures }, false) => {
                let failures = failures.saturating_add(1);
                if failures >= self.policy.failure_threshold {
                    tracing::warn!(failures, "circuit breaker opened");
                    State::Open {
                        until: now + self.policy.cool_down,
                    }
                } else {
                    State::Closed { failures }
                }
            }
            (State::HalfOpen { .. } | State::Open { .. }, false) => State::Open {
                until: now + self.policy.cool_down,
            },
        };
    }

    /// The current state.
    #[must_use]
    pub fn state(&self, now: Instant) -> BreakerState {
        let state = *self.lock();
        match state {
            State::Closed { .. } => BreakerState::Closed,
            State::Open { until } if now >= until => BreakerState::HalfOpen,
            State::Open { .. } => BreakerState::Open,
            State::HalfOpen { .. } => BreakerState::HalfOpen,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_probes_and_closes() {
        let policy = BreakerPolicy {
            failure_threshold: 2,
            cool_down: Duration::from_secs(5),
        };
        let b = CircuitBreaker::new(policy);
        let t0 = Instant::now();
        assert!(b.try_acquire(t0));
        b.record(false, t0);
        assert_eq!(b.state(t0), BreakerState::Closed);
        b.record(false, t0);
        assert_eq!(b.state(t0), BreakerState::Open);
        assert!(!b.try_acquire(t0));
        let later = t0 + Duration::from_secs(6);
        assert_eq!(b.state(later), BreakerState::HalfOpen);
        assert!(b.try_acquire(later));
        assert!(!b.try_acquire(later), "only one probe at a time");
        b.record(false, later);
        assert!(!b.try_acquire(later));
        let again = later + Duration::from_secs(6);
        assert!(b.try_acquire(again));
        b.record(true, again);
        assert_eq!(b.state(again), BreakerState::Closed);
        assert!(b.try_acquire(again));
    }

    #[test]
    fn caller_errors_do_not_count() {
        assert!(is_failure(ErrorKind::Timeout));
        assert!(!is_failure(ErrorKind::NotFound));
        assert!(!is_failure(ErrorKind::PermissionDenied));
    }
}
