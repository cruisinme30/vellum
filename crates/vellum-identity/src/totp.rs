// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Time-based one-time passwords (RFC 6238) for authenticator apps
//! (ADR 0069, D290).
//!
//! The parameters every authenticator app supports: HMAC-SHA-1, 6 digits,
//! 30-second steps, a 160-bit secret. A code is accepted for the current
//! step and one step either side, and only for a step later than the last
//! one accepted, so an observed code cannot be replayed.

use aws_lc_rs::hmac;
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind};

/// Seconds per step.
pub const PERIOD: u64 = 30;
/// Digits per code.
pub const DIGITS: u32 = 6;
/// Steps of clock drift tolerated either side.
const DRIFT: u64 = 1;

/// A new random 20-byte secret.
///
/// # Errors
///
/// [`ErrorKind::Internal`] if the random source fails.
pub fn generate_secret() -> Result<[u8; 20], Error> {
    let mut secret = [0_u8; 20];
    getrandom::fill(&mut secret)
        .map_err(|_| Error::new(ErrorKind::Internal, "random source failed"))?;
    Ok(secret)
}

/// The secret as authenticator apps take it: unpadded base32.
#[must_use]
pub fn encode_secret(secret: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(secret)
}

/// The `otpauth://` link an authenticator app scans.
#[must_use]
pub fn uri(secret: &[u8], issuer: &str, account: &str) -> String {
    let label = format!("{issuer}:{account}");
    let query = serde_urlencoded::to_string([
        ("secret", encode_secret(secret).as_str()),
        ("issuer", issuer),
        ("algorithm", "SHA1"),
        ("digits", "6"),
        ("period", "30"),
    ])
    .unwrap_or_default();
    format!(
        "otpauth://totp/{}?{query}",
        url::form_urlencoded::byte_serialize(label.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    )
}

/// The step a Unix time falls in.
#[must_use]
pub const fn step_at(unix_seconds: u64) -> u64 {
    unix_seconds / PERIOD
}

/// The code for one step (RFC 4226 dynamic truncation).
#[must_use]
pub fn code(secret: &[u8], step: u64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = hmac::sign(&key, &step.to_be_bytes());
    let mac = tag.as_ref();
    let offset = usize::from(mac.last().copied().unwrap_or(0) & 0x0f);
    let word = mac
        .get(offset..offset + 4)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map_or(0, u32::from_be_bytes)
        & 0x7fff_ffff;
    format!("{:06}", word % 10_u32.pow(DIGITS))
}

/// The step a presented code matches, if any: within the drift of `now`,
/// and later than `after` (the last step accepted).
#[must_use]
pub fn verify(secret: &[u8], presented: &str, now_step: u64, after: Option<u64>) -> Option<u64> {
    let presented: String = presented.chars().filter(|c| !c.is_whitespace()).collect();
    if presented.len() != 6 || !presented.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut found = None;
    // Every candidate is computed and compared, so timing says nothing
    // about which step matched.
    for step in now_step.saturating_sub(DRIFT)..=now_step.saturating_add(DRIFT) {
        let matches = bool::from(code(secret, step).as_bytes().ct_eq(presented.as_bytes()));
        if matches && after.is_none_or(|last| step > last) && found.is_none() {
            found = Some(step);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1, truncated to 6 digits.
    #[test]
    fn the_rfc_vectors_match() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(code(secret, step_at(time)), expected, "at {time}");
        }
    }

    #[test]
    fn codes_verify_once_within_the_drift() {
        let secret = b"12345678901234567890";
        let now = step_at(1_111_111_109);
        let current = code(secret, now);
        assert_eq!(verify(secret, &current, now, None), Some(now));
        assert_eq!(
            verify(
                secret,
                &format!("{} {}", &current[..3], &current[3..]),
                now,
                None
            ),
            Some(now)
        );
        // A replay of the step already used is refused.
        assert_eq!(verify(secret, &current, now, Some(now)), None);
        // One step of drift either way, not two.
        assert_eq!(
            verify(secret, &code(secret, now - 1), now, None),
            Some(now - 1)
        );
        assert_eq!(
            verify(secret, &code(secret, now + 1), now, None),
            Some(now + 1)
        );
        assert_eq!(verify(secret, &code(secret, now - 2), now, None), None);
        assert_eq!(verify(secret, "12345", now, None), None);
        assert_eq!(verify(secret, "abcdef", now, None), None);
    }

    #[test]
    fn the_link_names_the_account_and_secret() {
        let link = uri(b"12345678901234567890", "Vellum", "priya@northstar.dev");
        assert!(link.starts_with("otpauth://totp/Vellum%3Apriya%40northstar.dev?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Vellum"));
    }
}
