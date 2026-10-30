// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red-team regression tests for rate limits and uploads (ADR 0030, #273).

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use vellum_security::rate::Lockout;
use vellum_security::upload::safe_file_name;

const SEC: Duration = Duration::from_secs(1);

#[test]
fn a_flood_of_failing_keys_cannot_grow_the_lockout_table() {
    let lockout = Lockout::with_max_keys(3, SEC, 60 * SEC, 1_000);
    let now = Instant::now();
    for key in 0..100_000_u32 {
        let _ = lockout.fail(&key, now);
    }
    assert!(lockout.len() <= 1_000, "{} keys tracked", lockout.len());
}

#[test]
fn the_attackers_own_lockout_survives_a_flood() {
    let lockout = Lockout::with_max_keys(0, SEC, 3_600 * SEC, 100);
    let now = Instant::now();
    let attacker = u32::MAX;
    for _ in 0..10 {
        let _ = lockout.fail(&attacker, now);
    }
    // Fresh keys locked for one second each push the table over its limit.
    for key in 0..10_000_u32 {
        let _ = Lockout::fail(&lockout, &key, now);
    }
    assert!(
        lockout.check(&attacker, now).is_err(),
        "flood reset the lockout"
    );
}

#[test]
fn ended_lockouts_are_forgotten() {
    let lockout = Lockout::with_max_keys(0, SEC, 10 * SEC, 10);
    let start = Instant::now();
    for key in 0..10_u32 {
        let _ = lockout.fail(&key, start);
    }
    let later = start + 60 * SEC;
    let _ = lockout.fail(&100, later);
    assert_eq!(lockout.len(), 1);
}

#[test]
fn upload_names_lose_bidi_and_invisible_characters() {
    assert_eq!(safe_file_name("invoice\u{202e}fdp.txt"), "invoicefdp.txt");
    assert_eq!(safe_file_name("a\u{2066}b\u{2069}.png"), "ab.png");
    assert_eq!(safe_file_name("\u{feff}\u{200b}.env"), "env");
    assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_file_name("C:\\Windows\\evil.png"), "evil.png");
    assert_eq!(safe_file_name("\u{0}\u{7}"), "upload");
}
