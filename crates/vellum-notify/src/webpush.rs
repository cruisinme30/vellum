// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Web push: message encryption (RFC 8291) and VAPID (RFC 8292).
//!
//! The PWA and the desktop app register a push subscription: an endpoint
//! URL on the browser vendor's push service plus the browser's P-256 public
//! key and a 16-byte auth secret. Vellum encrypts every payload to that key
//! with the `aes128gcm` content coding (RFC 8188), so the push service
//! never sees notification text, and identifies itself with a VAPID JWT
//! signed by the server's own P-256 key. A 404 or 410 from the push
//! service removes the subscription (see [`crate::delivery::HealthAction`]).

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{Aead as _, KeyInit as _};
use hkdf::Hkdf;
use jiff::{SignedDuration, Timestamp};
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::{PublicKey, SecretKey};
use serde::Serialize;
use sha2::Sha256;
use url::Url;

use crate::encoding::{b64url, unb64url};
use crate::webhook::OutboundRequest;

/// The record size written into the header; payloads fit one record.
const RECORD_SIZE: u32 = 4096;
/// The largest plaintext push services accept in one message.
pub const MAX_PAYLOAD: usize = 3993;

/// A browser's push subscription, as `PushSubscription.toJSON()` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscription {
    /// The push service URL.
    pub endpoint: Url,
    /// The browser's uncompressed P-256 public key (`keys.p256dh`).
    pub p256dh: Vec<u8>,
    /// The 16-byte auth secret (`keys.auth`).
    pub auth: Vec<u8>,
}

/// Why a push message could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PushError {
    /// The subscription keys are not valid base64url P-256 / 16 bytes.
    #[error("invalid push subscription keys")]
    InvalidKeys,
    /// The payload is larger than [`MAX_PAYLOAD`].
    #[error("payload too large for one push message")]
    TooLarge,
    /// The system random source failed.
    #[error("the system random source failed")]
    Random,
    /// Key derivation or encryption failed.
    #[error("encryption failed")]
    Crypto,
}

impl PushSubscription {
    /// Builds a subscription from the base64url strings the browser sends.
    ///
    /// # Errors
    ///
    /// [`PushError::InvalidKeys`] for malformed keys.
    pub fn from_browser(endpoint: Url, p256dh: &str, auth: &str) -> Result<Self, PushError> {
        let p256dh = unb64url(p256dh).ok_or(PushError::InvalidKeys)?;
        let auth = unb64url(auth).ok_or(PushError::InvalidKeys)?;
        if PublicKey::from_sec1_bytes(&p256dh).is_err() || auth.len() != 16 {
            return Err(PushError::InvalidKeys);
        }
        Ok(Self {
            endpoint,
            p256dh,
            auth,
        })
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], PushError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| PushError::Random)?;
    Ok(bytes)
}

fn random_secret_key() -> Result<SecretKey, PushError> {
    // A random 32-byte string is a valid scalar with overwhelming
    // probability; retry the rare zero or out-of-range value.
    for _ in 0..8 {
        if let Ok(key) = SecretKey::from_slice(&random_bytes::<32>()?) {
            return Ok(key);
        }
    }
    Err(PushError::Random)
}

fn expand<const N: usize>(hkdf: &Hkdf<Sha256>, info: &[u8]) -> Result<[u8; N], PushError> {
    let mut out = [0_u8; N];
    hkdf.expand(info, &mut out).map_err(|_| PushError::Crypto)?;
    Ok(out)
}

/// Encrypts `payload` for `subscription` with a fresh key and salt.
///
/// # Errors
///
/// [`PushError`] for bad keys, oversize payloads or a failed RNG.
pub fn encrypt(subscription: &PushSubscription, payload: &[u8]) -> Result<Vec<u8>, PushError> {
    encrypt_with(
        subscription,
        payload,
        &random_secret_key()?,
        random_bytes::<16>()?,
    )
}

/// [`encrypt`] with a chosen sender key and salt, for the RFC test vector.
fn encrypt_with(
    subscription: &PushSubscription,
    payload: &[u8],
    sender: &SecretKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, PushError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(PushError::TooLarge);
    }
    let receiver =
        PublicKey::from_sec1_bytes(&subscription.p256dh).map_err(|_| PushError::InvalidKeys)?;
    let sender_public = sender.public_key().to_sec1_bytes();
    let shared = p256::ecdh::diffie_hellman(sender.to_nonzero_scalar(), receiver.as_affine());

    // RFC 8291 §3.3–3.4: combine the ECDH secret with the auth secret.
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&subscription.p256dh);
    key_info.extend_from_slice(&sender_public);
    let prk_key = Hkdf::<Sha256>::new(Some(&subscription.auth), shared.raw_secret_bytes());
    let ikm: [u8; 32] = expand(&prk_key, &key_info)?;

    // RFC 8188 §2.2–2.3: content-encryption key and nonce.
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let cek: [u8; 16] = expand(&prk, b"Content-Encoding: aes128gcm\0")?;
    let nonce: [u8; 12] = expand(&prk, b"Content-Encoding: nonce\0")?;

    let mut plaintext = payload.to_vec();
    plaintext.push(0x02); // delimiter of the last (only) record
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|_| PushError::Crypto)?;
    let ciphertext = cipher
        .encrypt(&nonce.into(), plaintext.as_slice())
        .map_err(|_| PushError::Crypto)?;

    let key_id_len = u8::try_from(sender_public.len()).map_err(|_| PushError::Crypto)?;
    let mut body = Vec::with_capacity(21 + sender_public.len() + ciphertext.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(key_id_len);
    body.extend_from_slice(&sender_public);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// The server's VAPID identity.
pub struct VapidKey {
    signing: SigningKey,
    /// `mailto:` or `https:` contact for push service operators.
    pub subject: String,
}

impl std::fmt::Debug for VapidKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VapidKey")
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    aud: &'a str,
    exp: i64,
    sub: &'a str,
}

impl VapidKey {
    /// Loads a key from its 32 secret bytes (stored sealed).
    ///
    /// # Errors
    ///
    /// [`PushError::InvalidKeys`] if the bytes are not a P-256 scalar.
    pub fn from_bytes(secret: &[u8], subject: String) -> Result<Self, PushError> {
        let signing = SigningKey::from_slice(secret).map_err(|_| PushError::InvalidKeys)?;
        Ok(Self { signing, subject })
    }

    /// Generates a new key, done once per installation.
    ///
    /// # Errors
    ///
    /// [`PushError::Random`] if the system random source fails.
    pub fn generate(subject: String) -> Result<Self, PushError> {
        let secret = random_secret_key()?;
        Self::from_bytes(&secret.to_bytes(), subject)
    }

    /// The secret bytes, for sealing at rest.
    #[must_use]
    pub fn secret_bytes(&self) -> Vec<u8> {
        self.signing.to_bytes().to_vec()
    }

    /// The public key as base64url, given to browsers as
    /// `applicationServerKey`.
    #[must_use]
    pub fn public_key(&self) -> String {
        let public = PublicKey::from(self.signing.verifying_key());
        b64url(&public.to_sec1_bytes())
    }

    /// The `Authorization` header value for an endpoint, valid 12 hours.
    ///
    /// # Errors
    ///
    /// [`PushError::InvalidKeys`] if the endpoint has no origin.
    pub fn authorization(&self, endpoint: &Url, now: Timestamp) -> Result<String, PushError> {
        let origin = endpoint.origin().ascii_serialization();
        if origin == "null" {
            return Err(PushError::InvalidKeys);
        }
        let exp = now
            .checked_add(SignedDuration::from_hours(12))
            .map_err(|_| PushError::Crypto)?
            .as_second();
        let claims = serde_json::to_vec(&Claims {
            aud: &origin,
            exp,
            sub: &self.subject,
        })
        .map_err(|_| PushError::Crypto)?;
        let signing_input = format!(
            "{}.{}",
            b64url(br#"{"typ":"JWT","alg":"ES256"}"#),
            b64url(&claims)
        );
        let signature: Signature = self.signing.sign(signing_input.as_bytes());
        Ok(format!(
            "vapid t={signing_input}.{}, k={}",
            b64url(&signature.to_bytes()),
            self.public_key()
        ))
    }
}

/// How quickly the push service should deliver (RFC 8030 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// Deliver when convenient for the device's battery.
    Normal,
    /// Deliver now: SLA breaches and reminders.
    High,
}

/// Builds the encrypted, VAPID-signed request for one push message.
///
/// `topic` makes the push service replace an undelivered message with the
/// same topic, so a burst of updates to one issue arrives as one.
///
/// # Errors
///
/// [`PushError`] from encryption or signing.
pub fn prepare(
    subscription: &PushSubscription,
    vapid: &VapidKey,
    payload: &[u8],
    topic: &str,
    urgency: Urgency,
    now: Timestamp,
    delivery_id: String,
) -> Result<OutboundRequest, PushError> {
    let body = encrypt(subscription, payload)?;
    // Topics are at most 32 base64url characters.
    let topic: String = topic
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    let mut headers = vec![
        (
            "Authorization".to_owned(),
            vapid.authorization(&subscription.endpoint, now)?,
        ),
        ("Content-Encoding".to_owned(), "aes128gcm".to_owned()),
        (
            "Content-Type".to_owned(),
            "application/octet-stream".to_owned(),
        ),
        ("TTL".to_owned(), "86400".to_owned()),
        (
            "Urgency".to_owned(),
            match urgency {
                Urgency::Normal => "normal",
                Urgency::High => "high",
            }
            .to_owned(),
        ),
    ];
    if !topic.is_empty() {
        headers.push(("Topic".to_owned(), topic));
    }
    Ok(OutboundRequest {
        url: subscription.endpoint.clone(),
        headers,
        body,
        delivery_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier as _;

    fn decode(text: &str) -> Vec<u8> {
        unb64url(text).unwrap_or_default()
    }

    fn rfc_subscription() -> Result<PushSubscription, PushError> {
        PushSubscription::from_browser(
            Url::parse("https://push.example.net/push/JzLQ3raZJfFBR0aqvOMsLrt54w4rJUsV")
                .map_err(|_| PushError::InvalidKeys)?,
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
            "BTBZMqHH6r4Tts7J_aSIgg",
        )
    }

    /// RFC 8291 Appendix A.
    #[test]
    fn rfc8291_test_vector() -> Result<(), PushError> {
        let sender = SecretKey::from_slice(&decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"))
            .map_err(|_| PushError::InvalidKeys)?;
        let salt: [u8; 16] = decode("DGv6ra1nlYgDCS1FRnbzlw")
            .try_into()
            .map_err(|_| PushError::InvalidKeys)?;
        let body = encrypt_with(
            &rfc_subscription()?,
            b"When I grow up, I want to be a watermelon",
            &sender,
            salt,
        )?;
        assert_eq!(
            b64url(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
        Ok(())
    }

    #[test]
    fn rejects_bad_keys_and_large_payloads() -> Result<(), PushError> {
        let endpoint =
            Url::parse("https://push.example.net/x").map_err(|_| PushError::InvalidKeys)?;
        assert_eq!(
            PushSubscription::from_browser(endpoint, "AAAA", "BTBZMqHH6r4Tts7J_aSIgg"),
            Err(PushError::InvalidKeys)
        );
        let big = vec![0_u8; MAX_PAYLOAD + 1];
        assert_eq!(
            encrypt(&rfc_subscription()?, &big),
            Err(PushError::TooLarge)
        );
        Ok(())
    }

    #[test]
    fn vapid_header_verifies() -> Result<(), PushError> {
        let vapid = VapidKey::generate("mailto:ops@northstar.dev".to_owned())?;
        let reloaded = VapidKey::from_bytes(&vapid.secret_bytes(), vapid.subject.clone())?;
        assert_eq!(reloaded.public_key(), vapid.public_key());

        let subscription = rfc_subscription()?;
        let request = prepare(
            &subscription,
            &vapid,
            br#"{"title":"ENG-142"}"#,
            "issue:ENG-142",
            Urgency::High,
            Timestamp::UNIX_EPOCH,
            "d1".to_owned(),
        )?;
        assert_eq!(request.header("Topic"), Some("issueENG-142"));
        assert_eq!(request.header("Urgency"), Some("high"));
        let auth = request.header("Authorization").unwrap_or_default();
        let (token, key) = auth
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .ok_or(PushError::InvalidKeys)?;
        assert_eq!(key, vapid.public_key());
        let (signing_input, signature) = token.rsplit_once('.').ok_or(PushError::InvalidKeys)?;
        let verifying =
            VerifyingKey::from_sec1_bytes(&decode(key)).map_err(|_| PushError::InvalidKeys)?;
        let signature =
            Signature::from_slice(&decode(signature)).map_err(|_| PushError::InvalidKeys)?;
        assert!(
            verifying
                .verify(signing_input.as_bytes(), &signature)
                .is_ok()
        );
        let claims = signing_input
            .split('.')
            .nth(1)
            .map(decode)
            .unwrap_or_default();
        let claims: serde_json::Value =
            serde_json::from_slice(&claims).map_err(|_| PushError::Crypto)?;
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["exp"], 43_200);
        Ok(())
    }
}
