// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Crockford base32, the alphabet used by ULIDs and block IDs.
//!
//! Encoding emits uppercase; decoding is case-insensitive and rejects `I`,
//! `L`, `O` and `U` rather than aliasing them, so spellings stay canonical.

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Returns the uppercase Crockford digit for the low five bits of `value`.
pub fn encode_digit(value: u8) -> char {
    char::from(
        ALPHABET
            .get(usize::from(value & 0x1f))
            .copied()
            .unwrap_or(b'0'),
    )
}

/// Returns the value of one Crockford digit, or `None` if `c` is not one.
pub const fn decode_digit(c: char) -> Option<u8> {
    let upper = c.to_ascii_uppercase();
    let value = match upper {
        '0'..='9' => upper as u32 - '0' as u32,
        'A'..='H' => upper as u32 - 'A' as u32 + 10,
        'J' | 'K' => upper as u32 - 'J' as u32 + 18,
        'M' | 'N' => upper as u32 - 'M' as u32 + 20,
        'P'..='T' => upper as u32 - 'P' as u32 + 22,
        'V'..='Z' => upper as u32 - 'V' as u32 + 27,
        _ => return None,
    };
    // `value` is always below 32, so the narrowing cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    Some(value as u8)
}

/// Encodes the low `5 * digits` bits of `value`, most significant digit first.
pub fn encode(value: u128, digits: u32) -> String {
    (0..digits)
        .rev()
        .map(|i| {
            // Masked to five bits, so the narrowing cannot truncate.
            #[allow(clippy::cast_possible_truncation)]
            let digit = ((value >> (5 * i)) & 0x1f) as u8;
            encode_digit(digit)
        })
        .collect()
}

/// Why a Crockford string failed to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The character at `position` is not a Crockford digit.
    Character { found: char, position: usize },
}

/// Decodes a Crockford string into an integer, without checking its length.
///
/// The caller bounds the length so the result cannot overflow `u128`.
pub fn decode(text: &str) -> Result<u128, DecodeError> {
    text.chars()
        .enumerate()
        .try_fold(0_u128, |acc, (position, found)| {
            decode_digit(found)
                .map(|digit| (acc << 5) | u128::from(digit))
                .ok_or(DecodeError::Character { found, position })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_digit_round_trips() {
        for value in 0..32_u8 {
            let c = encode_digit(value);
            assert_eq!(decode_digit(c), Some(value));
            assert_eq!(decode_digit(c.to_ascii_lowercase()), Some(value));
        }
    }

    #[test]
    fn excluded_letters_are_rejected() {
        for c in ['I', 'L', 'O', 'U', 'i', 'l', 'o', 'u', '-', ' ', 'é'] {
            assert_eq!(decode_digit(c), None, "{c:?} should be rejected");
        }
    }
}
