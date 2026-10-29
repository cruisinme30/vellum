// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! HMAC-SHA256 helpers for verifying inbound webhooks.

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;

/// HMAC-SHA256 over the concatenation of `parts`.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    let nibble = |n: u8| char::from_digit(u32::from(n), 16).unwrap_or('0');
    bytes
        .iter()
        .flat_map(|b| [nibble(b >> 4), nibble(b & 0x0f)])
        .collect()
}

/// Constant-time comparison of two byte strings.
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

/// Whether `signature_hex` is the hex HMAC of `parts` under `key`.
pub fn verify_hex(key: &[u8], parts: &[&[u8]], signature_hex: &str) -> bool {
    let expected = hex(&hmac_sha256(key, parts));
    equal(
        expected.as_bytes(),
        signature_hex.trim().to_ascii_lowercase().as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4231_case_2() {
        let mac = hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert!(verify_hex(
            b"Jefe",
            &[b"what do ya want for nothing?"],
            "5BDCC146BF60754E6A042426089575C75A003F089D2739839DEC58B964EC3843"
        ));
        assert!(!verify_hex(b"Jefe", &[b"x"], "00"));
    }
}
