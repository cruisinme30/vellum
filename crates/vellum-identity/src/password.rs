// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Password hashing with Argon2id.
//!
//! Parameters follow the OWASP recommendation (19 `MiB`, two passes, one
//! lane). Hashes are PHC strings, so the parameters can be raised later
//! and old hashes still verify; [`needs_rehash`] tells sign-in to upgrade
//! them.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use vellum_kernel::{Error, ErrorKind};

/// The shortest password accepted, in characters.
pub const MIN_LEN: usize = 12;
/// The longest password accepted, in bytes, to bound hashing work.
pub const MAX_LEN: usize = 1024;

const MEMORY_KIB: u32 = 19 * 1024;
const PASSES: u32 = 2;
const LANES: u32 = 1;

fn hasher() -> Result<Argon2<'static>, Error> {
    let params = Params::new(MEMORY_KIB, PASSES, LANES, None).map_err(|e| {
        Error::new(ErrorKind::Internal, format!("argon2 parameters: {e}"))
    })?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Checks a new password against the policy: 12 characters to 1024 bytes.
///
/// # Errors
///
/// [`ErrorKind::InvalidInput`] naming the rule that failed.
pub fn check_policy(password: &str) -> Result<(), Error> {
    if password.chars().count() < MIN_LEN {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("use a password of at least {MIN_LEN} characters"),
        ));
    }
    if password.len() > MAX_LEN {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("use a password of at most {MAX_LEN} bytes"),
        ));
    }
    Ok(())
}

/// Hashes a password into a PHC string.
///
/// # Errors
///
/// [`ErrorKind::Internal`] if the random source or hasher fails.
pub fn hash(password: &str) -> Result<String, Error> {
    let mut salt = [0_u8; 16];
    getrandom::fill(&mut salt).map_err(|e| {
        Error::new(ErrorKind::Internal, format!("random source failed: {e}"))
    })?;
    let hashed: PasswordHash = hasher()?
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|e| Error::new(ErrorKind::Internal, format!("argon2: {e}")))?;
    Ok(hashed.to_string())
}

/// Whether `password` matches the PHC string `stored`. A malformed hash
/// never matches.
#[must_use]
pub fn verify(password: &str, stored: &str) -> bool {
    if password.len() > MAX_LEN {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(stored) else {
        return false;
    };
    hasher()
        .is_ok_and(|h| h.verify_password(password.as_bytes(), &parsed).is_ok())
}

/// Spends the same work as a real verification, so signing in with an
/// unknown address takes as long as with a known one.
pub fn verify_dummy(password: &str) {
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
                         m2Ok0R8vzx5RjLK0XNbzX7/6ql4FLzqZcMf6yeNm4fA";
    let _ = verify(password, DUMMY);
}

/// Whether a stored hash uses weaker parameters than today's.
#[must_use]
pub fn needs_rehash(stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        return true;
    };
    let Ok(params) = Params::try_from(&parsed) else {
        return true;
    };
    parsed.algorithm.as_str() != "argon2id"
        || params.m_cost() < MEMORY_KIB
        || params.t_cost() < PASSES
}

/// How many password checks may run at once.
///
/// A sign-in flood then cannot take more than `cap` × 19 `MiB` of memory
/// or every CPU (red team round 4). A check that cannot start within a
/// short wait is refused as [`ErrorKind::RateLimited`] instead of queueing.
#[derive(Debug)]
pub struct Gate {
    cap: usize,
    wait: Duration,
    running: Mutex<usize>,
    freed: Condvar,
}

impl Default for Gate {
    /// Two checks per CPU, waiting at most half a second.
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map_or(2, std::num::NonZero::get);
        Self::new(
            cpus.saturating_mul(2).clamp(2, 64),
            Duration::from_millis(500),
        )
    }
}

impl Gate {
    /// At most `cap` checks at once, each waiting at most `wait` to start.
    #[must_use]
    pub fn new(cap: usize, wait: Duration) -> Self {
        Self {
            cap: cap.max(1),
            wait,
            running: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// A slot for one password check, held until the permit drops.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::RateLimited`] when every slot stays taken for the wait.
    pub fn permit(&self) -> Result<Permit<'_>, Error> {
        let deadline = Instant::now() + self.wait;
        let mut running =
            self.running.lock().unwrap_or_else(PoisonError::into_inner);
        while *running >= self.cap {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                tracing::warn!(
                    event = "identity.password_gate_full",
                    cap = self.cap,
                    "refused a password check: too many at once"
                );
                return Err(Error::new(
                    ErrorKind::RateLimited,
                    "the server is busy checking passwords; try again in a \
                        moment",
                ));
            }
            running = self
                .freed
                .wait_timeout(running, left)
                .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
        }
        *running += 1;
        drop(running);
        Ok(Permit(self))
    }
}

/// One running password check; see [`Gate::permit`].
#[derive(Debug)]
pub struct Permit<'a>(&'a Gate);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut running = self
            .0
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *running = running.saturating_sub(1);
        drop(running);
        self.0.freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_bounds_checks_and_refuses_quickly() {
        let gate = Gate::new(2, Duration::from_millis(100));
        let first = gate.permit().unwrap();
        let _second = gate.permit().unwrap();
        let started = Instant::now();
        let err = gate.permit().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RateLimited);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(first);
        let _third = gate.permit().unwrap();
    }

    #[test]
    fn a_waiting_check_starts_when_a_slot_frees() {
        let gate = std::sync::Arc::new(Gate::new(1, Duration::from_secs(5)));
        let held = std::sync::Arc::clone(&gate);
        let (tx, rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let permit = held.permit().unwrap();
            tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            drop(permit);
        });
        rx.recv().unwrap();
        assert!(gate.permit().is_ok());
        holder.join().unwrap();
    }

    #[test]
    fn hashes_verify_and_policy_holds() {
        let h = hash("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("correct horse battery", &h));
        assert!(!verify("wrong horse battery", &h));
        assert!(!verify("anything", "not a hash"));
        assert!(!needs_rehash(&h));
        assert!(needs_rehash(
            "$argon2id$v=19$m=1024,t=1,p=1$c29tZXNhbHQ$aGFzaGhhc2hoYXNo"
        ));
        assert!(check_policy("short").is_err());
        assert!(check_policy("long enough password").is_ok());
        assert!(check_policy(&"x".repeat(MAX_LEN + 1)).is_err());
        verify_dummy("whatever");
    }
}
