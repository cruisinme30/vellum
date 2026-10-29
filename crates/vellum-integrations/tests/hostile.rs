// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Hostile input for the inbound receivers (red team, issue #331).
//!
//! A provider secret that was never configured is an empty string. HMAC
//! accepts an empty key, so anyone could sign a forged delivery with it.
//! Every receiver must refuse all requests until a secret is set.

use hmac::{Hmac, KeyInit as _, Mac as _};
use jiff::Timestamp;
use sha2::Sha256;
use vellum_integrations::git::{github, gitlab};
use vellum_integrations::intake::{sentry, slack};

/// What an attacker computes: the hex HMAC of `parts` under an empty key.
fn forged(parts: &[&[u8]]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(b"") else {
        unreachable!("HMAC takes any key length")
    };
    for part in parts {
        mac.update(part);
    }
    let digits = b"0123456789abcdef";
    mac.finalize()
        .into_bytes()
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .map(|n| char::from(digits.get(usize::from(n)).copied().unwrap_or(b'0')))
        .collect()
}

#[test]
fn receivers_without_a_secret_refuse_forged_requests() {
    let body = br#"{"action":"closed","repository":{"full_name":"northstar/app"}}"#;

    let github_sig = format!("sha256={}", forged(&[body]));
    assert!(
        github::verify(b"", Some(&github_sig), body).is_err(),
        "GitHub accepted a delivery signed with an empty secret"
    );

    assert!(
        sentry::verify(b"", Some(&forged(&[body])), body).is_err(),
        "Sentry accepted a delivery signed with an empty secret"
    );

    let now = Timestamp::now();
    let ts = now.as_second().to_string();
    let slack_sig = format!("v0={}", forged(&[b"v0:", ts.as_bytes(), b":", body]));
    assert!(
        slack::verify(b"", Some(&ts), Some(&slack_sig), body, now).is_err(),
        "Slack accepted a request signed with an empty secret"
    );

    assert!(
        gitlab::verify("", Some("")).is_err(),
        "GitLab accepted an empty token"
    );
}
