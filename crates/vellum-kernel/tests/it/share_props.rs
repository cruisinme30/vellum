// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for share tokens.

use std::collections::HashSet;

use proptest::prelude::*;
use vellum_kernel::{ShareToken, ShareTokenError, ShareTokenHash};

proptest! {
    #[test]
    fn hash_hex_round_trips(bytes in any::<[u8; 32]>()) {
        let hash = ShareTokenHash::from_bytes(bytes);
        prop_assert_eq!(hash.to_string().parse::<ShareTokenHash>(), Ok(hash));
        prop_assert_eq!(
            hash.to_string().to_uppercase().parse::<ShareTokenHash>(),
            Ok(hash)
        );
    }

    #[test]
    fn valid_token_text_parses_and_verifies(text in "[0-9A-Za-z]{22}") {
        let token: ShareToken = text.parse().map_err(
            |e: ShareTokenError| TestCaseError::fail(e.to_string())
        )?;
        prop_assert_eq!(token.expose(), text.as_str());
        prop_assert!(token.hash().verify(&token));
    }

    #[test]
    fn parsing_arbitrary_text_never_panics(text in ".{0,80}") {
        let _ = text.parse::<ShareToken>();
        let _ = text.parse::<ShareTokenHash>();
    }
}

#[test]
fn generated_tokens_use_the_whole_alphabet_and_do_not_repeat()
-> Result<(), ShareTokenError> {
    let mut seen_chars = HashSet::new();
    let mut seen_tokens = HashSet::new();
    for _ in 0..2_000 {
        let token = ShareToken::generate()?;
        assert_eq!(token.expose().parse::<ShareToken>()?, token);
        seen_chars.extend(token.expose().chars());
        assert!(seen_tokens.insert(token.expose().to_owned()));
    }
    assert_eq!(seen_chars.len(), 62);
    Ok(())
}
