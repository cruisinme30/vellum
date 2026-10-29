// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! ULIDs for new help pages: validation and generation without a
//! dependency. Page IDs need to be unique, not unpredictable.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Crockford's base32 alphabet, as ULIDs use it.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Whether `text` is a canonical ULID: 26 characters of Crockford base32,
/// starting with 0 to 7 so the value fits in 128 bits.
#[must_use]
pub fn is_valid(text: &str) -> bool {
    text.len() == 26
        && text.bytes().all(|b| ALPHABET.contains(&b))
        && text
            .bytes()
            .next()
            .is_some_and(|b| (b'0'..=b'7').contains(&b))
}

/// A new ULID from the current time and `salt`, which keeps IDs created in
/// the same millisecond apart.
#[must_use]
pub fn generate(salt: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let state = RandomState::new();
    let mut high = state.build_hasher();
    high.write(salt.as_bytes());
    high.write_u8(1);
    let mut low = state.build_hasher();
    low.write(salt.as_bytes());
    low.write_u8(2);
    let random = (u128::from(high.finish()) << 16) ^ u128::from(low.finish());
    let random = random & ((1_u128 << 80) - 1);
    let value = ((millis & ((1_u128 << 48) - 1)) << 80) | random;
    encode(value)
}

/// Encodes a 128-bit value as 26 base32 characters.
fn encode(value: u128) -> String {
    (0..26)
        .rev()
        .map(|i| {
            let digit = usize::try_from((value >> (5 * i)) & 31).unwrap_or(0);
            ALPHABET.get(digit).map_or('0', |b| char::from(*b))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_valid_and_distinct() {
        let a = generate("user/README.md");
        let b = generate("user/glossary.md");
        assert!(is_valid(&a), "{a}");
        assert!(is_valid(&b), "{b}");
        assert_ne!(a, b);
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(is_valid("01JB3K9Q7Z5N8M2X4V6T0R1Y3W"));
        assert!(!is_valid("01JB3K9Q7Z5N8M2X4V6T0R1Y3"));
        assert!(!is_valid("81JB3K9Q7Z5N8M2X4V6T0R1Y3W"));
        assert!(!is_valid("01JB3K9Q7Z5N8M2X4V6T0R1Y3U"));
    }
}
