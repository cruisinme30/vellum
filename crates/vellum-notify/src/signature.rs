// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! HMAC-SHA256 webhook signatures.
//!
//! Every delivery carries
//!
//! ```text
//! Vellum-Signature: t=1767225600,v1=2547b9d90518426b3b657b66b5d2f7e0aeb1200f474ce88c2f4b24e7fd352f31
//! ```
//!
//! where `v1` is the lower-case hex HMAC-SHA256, keyed by the webhook
//! secret, of `"{t}.{raw body}"`. Binding the timestamp into the MAC stops
//! replays outside the tolerance window. During secret rotation Vellum
//! signs with both the old and the new secret, so the header carries two
//! `v1` entries and a receiver accepts either (D80).

use hmac::{Hmac, KeyInit as _, Mac as _};
use jiff::{SignedDuration, Timestamp};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;

use crate::encoding::{hex, unhex};
use crate::secret::WebhookSecret;

/// The HTTP header that carries the signature.
pub const SIGNATURE_HEADER: &str = "Vellum-Signature";

/// How far a signature's timestamp may be from the receiver's clock.
pub const DEFAULT_TOLERANCE: SignedDuration = SignedDuration::from_mins(5);

/// Why a signature was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// The header is not `t=…,v1=…`.
    #[error("malformed signature header")]
    Malformed,
    /// The timestamp is outside the tolerance window.
    #[error("signature timestamp is outside the tolerance window")]
    Stale,
    /// No `v1` value matches any secret.
    #[error("signature does not match")]
    Mismatch,
}

fn mac(secret: &WebhookSecret, timestamp: i64, body: &[u8]) -> [u8; 32] {
    // HMAC accepts keys of any length, so `new_from_slice` cannot fail here.
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.finalize().into_bytes().into()
}

/// Builds the header value for a body signed at `at` with every secret
/// in `secrets` (one normally, two during rotation).
#[must_use]
pub fn sign(secrets: &[&WebhookSecret], at: Timestamp, body: &[u8]) -> String {
    let t = at.as_second();
    let mut header = format!("t={t}");
    for secret in secrets {
        header.push_str(",v1=");
        header.push_str(&hex(&mac(secret, t, body)));
    }
    header
}

/// Checks a header against the body, the secret and the receiver's clock.
///
/// # Errors
///
/// [`SignatureError`] says why the delivery must be refused.
pub fn verify(
    header: &str,
    body: &[u8],
    secret: &WebhookSecret,
    now: Timestamp,
    tolerance: SignedDuration,
) -> Result<(), SignatureError> {
    let mut timestamp = None;
    let mut candidates = Vec::new();
    for part in header.split(',') {
        let (key, value) = part
            .trim()
            .split_once('=')
            .ok_or(SignatureError::Malformed)?;
        match key {
            "t" => {
                timestamp = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| SignatureError::Malformed)?,
                );
            }
            "v1" => candidates.push(unhex(value).ok_or(SignatureError::Malformed)?),
            // Unknown schemes are ignored so a future v2 can be added.
            _ => {}
        }
    }
    let t = timestamp.ok_or(SignatureError::Malformed)?;
    if candidates.is_empty() {
        return Err(SignatureError::Malformed);
    }
    let signed_at = Timestamp::from_second(t).map_err(|_| SignatureError::Malformed)?;
    if now.duration_since(signed_at).abs() > tolerance {
        return Err(SignatureError::Stale);
    }
    let expected = mac(secret, t, body);
    if candidates
        .iter()
        .any(|candidate| bool::from(candidate.as_slice().ct_eq(&expected)))
    {
        Ok(())
    } else {
        Err(SignatureError::Mismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    #[test]
    fn known_answer() {
        // HMAC-SHA256(key = 32 × 0x0b, "1767225600.{}") computed with
        // `openssl dgst -sha256 -mac HMAC -macopt hexkey:0b…0b`.
        let secret = WebhookSecret::from_bytes([0x0b; 32]);
        let header = sign(&[&secret], at(1_767_225_600), b"{}");
        assert_eq!(
            header,
            "t=1767225600,v1=2547b9d90518426b3b657b66b5d2f7e0aeb1200f474ce88c2f4b24e7fd352f31"
        );
    }

    #[test]
    fn sign_then_verify() {
        let secret = WebhookSecret::from_bytes([1; 32]);
        let header = sign(&[&secret], at(1_000), b"payload");
        assert_eq!(
            verify(&header, b"payload", &secret, at(1_100), DEFAULT_TOLERANCE),
            Ok(())
        );
        assert_eq!(
            verify(&header, b"tampered", &secret, at(1_100), DEFAULT_TOLERANCE),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify(&header, b"payload", &secret, at(2_000), DEFAULT_TOLERANCE),
            Err(SignatureError::Stale)
        );
    }

    #[test]
    fn rotation_signs_with_both_secrets() {
        let old = WebhookSecret::from_bytes([1; 32]);
        let new = WebhookSecret::from_bytes([2; 32]);
        let header = sign(&[&old, &new], at(5), b"x");
        assert_eq!(header.matches("v1=").count(), 2);
        assert!(verify(&header, b"x", &old, at(5), DEFAULT_TOLERANCE).is_ok());
        assert!(verify(&header, b"x", &new, at(5), DEFAULT_TOLERANCE).is_ok());
    }

    #[test]
    fn malformed_headers() {
        let secret = WebhookSecret::from_bytes([1; 32]);
        for header in ["", "t=1", "v1=00", "t=x,v1=00", "t=1,v1=zz", "garbage"] {
            assert_eq!(
                verify(header, b"", &secret, at(1), DEFAULT_TOLERANCE),
                Err(SignatureError::Malformed),
                "{header}"
            );
        }
    }
}
