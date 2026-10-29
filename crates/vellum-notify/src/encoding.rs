// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Small encoders shared by the signing and push modules.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

/// Lower-case hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    let nibble = |n: u8| char::from_digit(u32::from(n), 16).unwrap_or('0');
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble(byte >> 4));
        out.push(nibble(byte & 0x0f));
    }
    out
}

/// Decodes hexadecimal of either case; `None` on odd length or bad digits.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    const fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks_exact(2)
        .map(|pair| match pair {
            [high, low] => Some(digit(*high)? << 4 | digit(*low)?),
            _ => None,
        })
        .collect()
}

/// Base64url without padding (RFC 4648 §5), as web push and JWTs use.
pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url, tolerating trailing padding.
pub fn unb64url(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// Standard padded base64, as MIME bodies use.
pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00, 0x7f, 0xab, 0xff];
        assert_eq!(hex(&bytes), "007fabff");
        assert_eq!(unhex("007FABff"), Some(bytes.to_vec()));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn base64url_tolerates_padding() {
        assert_eq!(b64url(b"hi?"), "aGk_");
        assert_eq!(unb64url("aGk_"), Some(b"hi?".to_vec()));
        assert_eq!(unb64url("aGk="), Some(b"hi".to_vec()));
        assert_eq!(b64(b"hi"), "aGk=");
    }
}
