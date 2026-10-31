// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secret scanning on save.
//!
//! Test tokens are assembled at run time so the repository's own secret
//! scanner never sees a complete token in the source.

#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use vellum_security::secrets::{ALLOW_MARKER, Rule, SecretPolicy, scan};

const UPPER_DIGIT: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const ALNUM: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// A deterministic, well-mixed string of `n` characters from `chars`.
fn body(chars: &str, n: usize) -> String {
    let chars: Vec<char> = chars.chars().collect();
    (0..n)
        .map(|i| *chars.get((i * 7 + i * i * 3 + 5) % chars.len()).unwrap())
        .collect()
}

fn token(prefix: &[&str], chars: &str, n: usize) -> String {
    format!("{}{}", prefix.concat(), body(chars, n))
}

fn rules(text: &str) -> Vec<Rule> {
    scan(text).into_iter().map(|f| f.rule).collect()
}

#[test]
fn finds_provider_tokens() {
    let cases = [
        (token(&["AK", "IA"], UPPER_DIGIT, 16), Rule::AwsAccessKey),
        (token(&["gh", "p_"], ALNUM, 36), Rule::GitHubToken),
        (token(&["github", "_pat_"], ALNUM, 82), Rule::GitHubToken),
        (token(&["gl", "pat-"], ALNUM, 20), Rule::GitLabToken),
        (token(&["xo", "xb-"], ALNUM, 40), Rule::SlackToken),
        (token(&["sk_", "live_"], ALNUM, 24), Rule::StripeKey),
        (token(&["AI", "za"], ALNUM, 35), Rule::GoogleApiKey),
        (token(&["sk-", "ant-"], ALNUM, 90), Rule::AnthropicKey),
        (token(&["sk-", "proj-"], ALNUM, 48), Rule::OpenAiKey),
        (token(&["np", "m_"], ALNUM, 36), Rule::NpmToken),
    ];
    for (secret, rule) in cases {
        let text =
            format!("Northstar deploy notes\n\nUse `{secret}` for staging.\n");
        let found = scan(&text);
        assert_eq!(found.len(), 1, "{rule:?}");
        let f = &found[0];
        assert_eq!(f.rule, rule);
        assert_eq!(&text[f.range.clone()], secret);
        assert_eq!((f.line, f.column), (3, 6));
        assert!(!f.redacted.contains(&secret[4..]));
        assert!(!format!("{f:?}").contains(&secret[4..]));
    }
}

#[test]
fn ignores_near_misses() {
    let too_short = token(&["gh", "p_"], ALNUM, 20);
    let embedded = format!("x{}", token(&["AK", "IA"], UPPER_DIGIT, 16));
    let too_long = token(&["AK", "IA"], UPPER_DIGIT, 20);
    let lower = token(&["AK", "IA"], "abcdef0123", 16);
    for text in [too_short, embedded, too_long, lower] {
        assert!(rules(&text).is_empty(), "{text}");
    }
}

#[test]
fn finds_private_keys_and_jwts() {
    let pem = format!("-----BEGIN {}PRIVATE KEY-----\nMIIE...\n", "RSA ");
    assert_eq!(rules(&pem), [Rule::PrivateKey]);
    let public = format!("-----BEGIN {}-----\n", "PUBLIC KEY");
    assert!(rules(&public).is_empty());

    let jwt = format!(
        "{}.{}.{}",
        token(&["ey", "J"], ALNUM, 20),
        token(&["ey", "J"], ALNUM, 40),
        body(ALNUM, 43)
    );
    assert_eq!(rules(&format!("Authorization: Bearer {jwt}")), [Rule::Jwt]);
    let not_jwt =
        format!("{}.{}", token(&["ey", "J"], ALNUM, 20), body(ALNUM, 20));
    assert!(rules(&not_jwt).is_empty());
}

#[test]
fn finds_high_entropy_assignments() {
    let value = body(ALNUM, 32);
    for name in ["api_key", "DB_PASSWORD", "client_secret", "\"token\""] {
        for sep in [" = ", ": ", "=\"", ": '"] {
            let text = format!("{name}{sep}{value}");
            assert_eq!(rules(&text), [Rule::Assignment], "{text}");
        }
    }
    // Low entropy, placeholders and plain prose are fine.
    for text in [
        format!("{} = {}", "password", "aaaaaaaaaaaaaaaaaaaa"),
        format!("{} = {}", "api_key", "YOUR_API_KEY_GOES_HERE_1234"),
        format!("{} = {}", "token", "${NORTHSTAR_TOKEN_FROM_ENV}"),
        format!("{}: {}", "tokens", value),
        "The password must be at least 12 characters.".to_owned(),
    ] {
        assert!(rules(&text).is_empty(), "{text}");
    }
}

#[test]
fn allow_marker_skips_a_line() {
    let secret = token(&["gh", "p_"], ALNUM, 36);
    let text = format!("`{secret}` <!-- {ALLOW_MARKER} -->\n`{secret}`\n");
    let found = scan(&text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, 2);
}

#[test]
fn columns_count_characters_and_ranges_are_absolute() {
    let secret = token(&["np", "m_"], ALNUM, 36);
    let text = format!("é\nÜber {secret}\n");
    let f = &scan(&text)[0];
    assert_eq!((f.line, f.column), (2, 6));
    assert_eq!(&text[f.range.clone()], secret);
}

#[test]
fn policy_decides_the_outcome() {
    let text = format!("key {}", token(&["AK", "IA"], UPPER_DIGIT, 16));
    assert!(SecretPolicy::Off.check(&text).unwrap().is_empty());
    assert_eq!(SecretPolicy::Warn.check(&text).unwrap().len(), 1);
    assert_eq!(SecretPolicy::default(), SecretPolicy::Warn);
    let err = SecretPolicy::Block.check(&text).unwrap_err();
    assert_eq!(err.findings[0].rule.id(), "aws-access-key");
    assert!(
        SecretPolicy::Block
            .check("# Northstar roadmap")
            .unwrap()
            .is_empty()
    );
}

proptest! {
    #[test]
    fn never_panics_and_ranges_are_valid(text in "\\PC{0,400}") {
        for f in scan(&text) {
            prop_assert!(text.get(f.range.clone()).is_some());
            prop_assert!(f.line >= 1 && f.column >= 1);
        }
    }

    #[test]
    fn prose_is_clean(words in prop::collection::vec("[a-z]{1,10}", 0..60)) {
        prop_assert!(scan(&words.join(" ")).is_empty());
    }
}
