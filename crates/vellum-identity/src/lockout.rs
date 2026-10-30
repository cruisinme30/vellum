// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in lockout: exponential back-off per address and per client IP.
//!
//! A single process counts in memory ([`Lockout`]). When the store is
//! shared, the counters live in the store too, so N replicas allow the
//! same number of attempts as one (ADR 0037, D160). Both follow the same
//! policy: some free failures, then a wait that doubles from 1 second up
//! to 15 minutes.

use std::fmt::Write as _;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use vellum_kernel::Timestamp;
use vellum_security::rate::{Lockout, client_bucket};

use crate::store::{Attempts, Store};

const BASE: Duration = Duration::from_secs(1);
const MAX: Duration = Duration::from_mins(15);
const EMAIL_FREE: u32 = 5;
const IP_FREE: u32 = 20;

/// What a lockout counts failures against.
#[derive(Debug, Clone, Copy)]
pub enum Key<'a> {
    /// A sign-in address (already lower-cased).
    Email(&'a str),
    /// A client IP.
    Ip(IpAddr),
}

impl Key<'_> {
    const fn free(self) -> u32 {
        match self {
            Self::Email(_) => EMAIL_FREE,
            Self::Ip(_) => IP_FREE,
        }
    }

    /// The record ID: a hash, so the table holds no addresses.
    fn id(self) -> String {
        let text = match self {
            Self::Email(email) => format!("email:{email}"),
            Self::Ip(ip) => format!("ip:{}", client_bucket(ip)),
        };
        Sha256::digest(text.as_bytes())
            .iter()
            .fold(String::with_capacity(64), |mut hex, b| {
                let _ = write!(hex, "{b:02x}");
                hex
            })
    }
}

/// The wait after `failures` failures with `free` free ones.
fn wait(failures: u32, free: u32) -> Duration {
    let over = failures.saturating_sub(free);
    if over == 0 {
        return Duration::ZERO;
    }
    let factor = 1_u32.checked_shl(over - 1).unwrap_or(u32::MAX);
    BASE.saturating_mul(factor).min(MAX)
}

fn plus(at: Timestamp, span: Duration) -> Timestamp {
    crate::identity::after(at, span)
}

/// The lockout counters of one identity service.
#[derive(Debug)]
pub struct Lockouts {
    email: Lockout<String>,
    ip: Lockout<IpAddr>,
}

impl Lockouts {
    pub fn new() -> Self {
        Self {
            email: Lockout::new(EMAIL_FREE, BASE, MAX),
            ip: Lockout::new(IP_FREE, BASE, MAX),
        }
    }

    /// Whether `key` may try now; otherwise how long until it may.
    pub fn check(&self, store: &Store, key: Key<'_>, now: Instant) -> Result<(), Duration> {
        if store.is_shared() {
            let id = key.id();
            let until = store.read(|s| s.lockouts.get(&id).map(|a| a.until));
            let now = Timestamp::now();
            return match until {
                Some(until) if until > now => Err(Duration::from_millis(
                    u64::try_from(until.unix_ms() - now.unix_ms()).unwrap_or(0),
                )),
                _ => Ok(()),
            };
        }
        match key {
            Key::Email(email) => self.email.check(&email.to_owned(), now),
            Key::Ip(ip) => self.ip.check(&client_bucket(ip), now),
        }
        .map_err(|l| l.retry_after)
    }

    /// Records a failure.
    pub fn fail(&self, store: &Store, key: Key<'_>, now: Instant) {
        if store.is_shared() {
            let id = key.id();
            let free = key.free();
            let now = Timestamp::now();
            let written = store.write(|s| {
                if !s.lockouts.contains_key(&id) {
                    s.bound_lockouts(now);
                }
                let failures = s
                    .lockouts
                    .get(&id)
                    .map_or(0, |a| a.failures)
                    .saturating_add(1);
                let _ = s.lockouts.insert(
                    id,
                    Attempts {
                        failures,
                        until: plus(now, wait(failures, free)),
                    },
                );
            });
            if let Err(error) = written {
                tracing::warn!(%error, "a failed sign-in was not counted");
            }
            return;
        }
        let _ = match key {
            Key::Email(email) => self.email.fail(&email.to_owned(), now),
            Key::Ip(ip) => self.ip.fail(&client_bucket(ip), now),
        };
    }

    /// Clears `key` after a success.
    pub fn succeed(&self, store: &Store, key: Key<'_>) {
        if store.is_shared() {
            let id = key.id();
            // Most sign-ins have nothing to clear: skip the write.
            if store.read(|s| s.lockouts.contains_key(&id)) {
                let _ = store.write(|s| s.lockouts.remove(&id));
            }
            return;
        }
        match key {
            Key::Email(email) => self.email.succeed(&email.to_owned()),
            Key::Ip(ip) => self.ip.succeed(&client_bucket(ip)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_after_the_free_failures() {
        assert_eq!(wait(5, 5), Duration::ZERO);
        assert_eq!(wait(6, 5), Duration::from_secs(1));
        assert_eq!(wait(8, 5), Duration::from_secs(4));
        assert_eq!(wait(200, 5), MAX);
    }

    #[test]
    fn keys_are_hashed() {
        let id = Key::Email("priya@northstar.dev").id();
        assert_eq!(id.len(), 64);
        assert!(!id.contains("priya"));
        assert_ne!(id, Key::Ip("10.0.0.1".parse().unwrap()).id());
    }

    /// Red team round 4: rotating addresses inside one `IPv6` `/64` must not
    /// reset the IP lockout.
    #[test]
    fn an_ipv6_client_is_locked_out_across_its_slash_64() {
        let lockouts = Lockouts::new();
        let store = Store::memory();
        let now = Instant::now();
        for n in 0..=IP_FREE {
            let ip = format!("2001:db8:7:7::{n:x}").parse().unwrap();
            lockouts.fail(&store, Key::Ip(ip), now);
        }
        let fresh = "2001:db8:7:7:dead:beef::1".parse().unwrap();
        assert!(lockouts.check(&store, Key::Ip(fresh), now).is_err());
        let neighbour = "2001:db8:7:8::1".parse().unwrap();
        assert!(lockouts.check(&store, Key::Ip(neighbour), now).is_ok());
        assert_eq!(
            Key::Ip("2001:db8:7:7::1".parse().unwrap()).id(),
            Key::Ip("2001:db8:7:7::2".parse().unwrap()).id()
        );
    }
}
