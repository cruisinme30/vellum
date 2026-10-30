// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end encrypted envelopes for E2E private spaces (format version 2).
//!
//! The payload is sealed exactly as in version 1, with one fresh DEK. The DEK
//! is wrapped once per member: an ephemeral X25519 key agrees a secret with
//! the member's public key, HKDF-SHA256 turns it into a one-off KEK, and
//! XChaCha20-Poly1305 wraps the DEK. Only members' secret keys open it, so
//! the server stores ciphertext it cannot read.
//!
//! | bytes      | field                                          |
//! |------------|------------------------------------------------|
//! | 4          | magic `\0VLE`                                   |
//! | 1          | format version, `2`                            |
//! | 1          | recipient count `n` (1–255)                    |
//! | n × 120    | stanzas: fingerprint 16, ephemeral key 32, nonce 24, wrapped DEK 48 |
//! | 24         | payload nonce                                  |
//! | rest       | payload ciphertext + 16-byte tag               |
//!
//! Adding a member wraps the same DEK once more; removing one drops a
//! stanza. Neither touches the payload. Removal stops future reads of this
//! object's file only; a removed member who kept the DEK can still read
//! this version, so content written after removal gets fresh DEKs.

use std::fmt;

use curve25519_dalek::montgomery::MontgomeryPoint;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::aead::{self, NONCE_LEN, TAG_LEN};
use crate::envelope::MAGIC;
use crate::error::CryptoError;
use crate::key::{KEY_LEN, SecretKey};

/// Format version of recipient envelopes.
pub const VERSION: u8 = 2;
/// Most members one envelope can be sealed to.
pub const MAX_RECIPIENTS: usize = 255;
/// Length of a recipient [`Fingerprint`].
pub const FINGERPRINT_LEN: usize = 16;

const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;
const STANZA_LEN: usize = FINGERPRINT_LEN + KEY_LEN + NONCE_LEN + WRAPPED_LEN;
const HKDF_INFO: &[u8] = b"vellum/x25519/v1";
const PAYLOAD_AAD: &[u8] = b"vellum/blob/v2";

/// A member's X25519 secret key. Lives only on the member's devices; wiped
/// on drop and never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct RecipientSecret([u8; KEY_LEN]);

impl RecipientSecret {
    /// Generates a fresh key from the system CSPRNG.
    ///
    /// # Errors
    /// [`CryptoError::Rng`] if the random source fails.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0_u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|_| CryptoError::Rng)?;
        Ok(Self(bytes))
    }

    /// Restores a key the member stored.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw key, for the member's own secure storage only.
    #[must_use]
    pub const fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// The matching public key, shared with the server's member registry.
    #[must_use]
    pub fn public(&self) -> RecipientPublic {
        RecipientPublic(MontgomeryPoint::mul_base_clamped(self.0).to_bytes())
    }
}

impl fmt::Debug for RecipientSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecipientSecret(..)")
    }
}

/// A member's X25519 public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecipientPublic([u8; KEY_LEN]);

impl RecipientPublic {
    /// Wraps public key bytes from the member registry.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw public key.
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; KEY_LEN] {
        self.0
    }

    /// The short id stored in each stanza: the first 16 bytes of SHA-256 of
    /// the public key.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        let digest = Sha256::digest(self.0);
        let mut out = [0_u8; FINGERPRINT_LEN];
        for (slot, byte) in out.iter_mut().zip(digest.iter()) {
            *slot = *byte;
        }
        Fingerprint(out)
    }
}

/// Identifies which member a stanza is for, without revealing the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint([u8; FINGERPRINT_LEN]);

impl Fingerprint {
    /// The raw fingerprint bytes.
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; FINGERPRINT_LEN] {
        self.0
    }
}

/// True if `bytes` is a recipient (version 2) envelope.
#[must_use]
pub fn is_recipient_envelope(bytes: &[u8]) -> bool {
    bytes.strip_prefix(&MAGIC).and_then(<[u8]>::first) == Some(&VERSION)
}

/// Seals `plaintext` so that each of `recipients` can open it.
///
/// # Errors
/// [`CryptoError::Recipients`] for an empty or oversized recipient list or a
/// degenerate public key; RNG or AEAD failures.
#[tracing::instrument(
    name = "crypto.seal_for",
    skip_all,
    fields(bytes = plaintext.len(), recipients = recipients.len())
)]
pub fn seal_for(
    recipients: &[RecipientPublic],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut unique: Vec<RecipientPublic> = Vec::with_capacity(recipients.len());
    for r in recipients {
        if !unique.contains(r) {
            unique.push(*r);
        }
    }
    let dek = SecretKey::generate()?;
    let stanzas = unique
        .iter()
        .map(|r| wrap_to(r, &dek))
        .collect::<Result<Vec<_>, _>>()?;
    let (nonce, ciphertext) = aead::seal(&dek, PAYLOAD_AAD, plaintext)?;
    encode(&stanzas, &nonce, &ciphertext)
}

/// Opens a recipient envelope with a member's secret key.
///
/// # Errors
/// [`CryptoError::Recipients`] if the key is not a recipient,
/// [`CryptoError::Decrypt`] on tampering, [`CryptoError::Malformed`] for bad
/// bytes.
#[tracing::instrument(
    name = "crypto.open_as",
    skip_all,
    fields(bytes = sealed.len())
)]
pub fn open_as(
    secret: &RecipientSecret,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let parsed = decode(sealed)?;
    let dek = unwrap_for(secret, &parsed.stanzas)?;
    aead::open(&dek, parsed.nonce, PAYLOAD_AAD, parsed.ciphertext)
}

/// Adds `member` as a recipient. `secret` must belong to a current
/// recipient. Returns the input unchanged if `member` already is one.
///
/// # Errors
/// As [`open_as`] for the DEK, and [`CryptoError::Recipients`] when the
/// envelope already has [`MAX_RECIPIENTS`].
pub fn add_recipient(
    secret: &RecipientSecret,
    member: &RecipientPublic,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let parsed = decode(sealed)?;
    let fingerprint = member.fingerprint();
    if parsed.stanzas.iter().any(|s| s.fingerprint == fingerprint) {
        return Ok(sealed.to_vec());
    }
    let dek = unwrap_for(secret, &parsed.stanzas)?;
    let mut stanzas = parsed.stanzas;
    stanzas.push(wrap_to(member, &dek)?);
    encode(&stanzas, parsed.nonce, parsed.ciphertext)
}

/// Removes `member`'s stanza. Needs no key.
///
/// # Errors
/// [`CryptoError::Recipients`] if `member` is the last recipient;
/// [`CryptoError::Malformed`] for bad bytes.
pub fn remove_recipient(
    member: &RecipientPublic,
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let parsed = decode(sealed)?;
    let fingerprint = member.fingerprint();
    let stanzas: Vec<Stanza> = parsed
        .stanzas
        .into_iter()
        .filter(|s| s.fingerprint != fingerprint)
        .collect();
    encode(&stanzas, parsed.nonce, parsed.ciphertext)
}

/// Fingerprints of every recipient, read from the header.
///
/// # Errors
/// [`CryptoError::Malformed`] for bad bytes.
pub fn recipients_of(sealed: &[u8]) -> Result<Vec<Fingerprint>, CryptoError> {
    Ok(decode(sealed)?
        .stanzas
        .iter()
        .map(|s| s.fingerprint)
        .collect())
}

struct Stanza {
    fingerprint: Fingerprint,
    ephemeral: [u8; KEY_LEN],
    nonce: [u8; NONCE_LEN],
    wrapped: Vec<u8>,
}

struct Parsed<'a> {
    stanzas: Vec<Stanza>,
    nonce: &'a [u8; NONCE_LEN],
    ciphertext: &'a [u8],
}

fn stanza_key(
    shared: &MontgomeryPoint,
    ephemeral: &[u8; KEY_LEN],
    recipient: &[u8; KEY_LEN],
) -> Result<SecretKey, CryptoError> {
    let shared = Zeroizing::new(shared.to_bytes());
    // A low-order public key yields the all-zero secret; refuse it.
    if shared.iter().all(|b| *b == 0) {
        return Err(CryptoError::Recipients("degenerate public key"));
    }
    let salt = [ephemeral.as_slice(), recipient.as_slice()].concat();
    let mut kek = Zeroizing::new([0_u8; KEY_LEN]);
    Hkdf::<Sha256>::new(Some(&salt), shared.as_slice())
        .expand(HKDF_INFO, kek.as_mut())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(SecretKey::from_bytes(*kek))
}

fn wrap_to(
    recipient: &RecipientPublic,
    dek: &SecretKey,
) -> Result<Stanza, CryptoError> {
    let ephemeral_secret = RecipientSecret::generate()?;
    let ephemeral = ephemeral_secret.public().0;
    let shared = MontgomeryPoint(recipient.0).mul_clamped(ephemeral_secret.0);
    let kek = stanza_key(&shared, &ephemeral, &recipient.0)?;
    let fingerprint = recipient.fingerprint();
    let (nonce, wrapped) = aead::seal(&kek, &fingerprint.0, dek.expose())?;
    Ok(Stanza {
        fingerprint,
        ephemeral,
        nonce,
        wrapped,
    })
}

fn unwrap_for(
    secret: &RecipientSecret,
    stanzas: &[Stanza],
) -> Result<SecretKey, CryptoError> {
    let public = secret.public();
    let fingerprint = public.fingerprint();
    let stanza = stanzas
        .iter()
        .find(|s| s.fingerprint == fingerprint)
        .ok_or(CryptoError::Recipients("not a recipient of this envelope"))?;
    let shared = MontgomeryPoint(stanza.ephemeral).mul_clamped(secret.0);
    let kek = stanza_key(&shared, &stanza.ephemeral, &public.0)?;
    let dek = Zeroizing::new(aead::open(
        &kek,
        &stanza.nonce,
        &fingerprint.0,
        &stanza.wrapped,
    )?);
    SecretKey::from_slice(&dek)
}

fn encode(
    stanzas: &[Stanza],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if stanzas.is_empty() {
        return Err(CryptoError::Recipients(
            "an envelope needs at least one recipient",
        ));
    }
    let count = u8::try_from(stanzas.len())
        .map_err(|_| CryptoError::Recipients("more than 255 recipients"))?;
    let mut out = Vec::with_capacity(
        6 + stanzas.len() * STANZA_LEN + NONCE_LEN + ciphertext.len(),
    );
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(count);
    for s in stanzas {
        out.extend_from_slice(&s.fingerprint.0);
        out.extend_from_slice(&s.ephemeral);
        out.extend_from_slice(&s.nonce);
        out.extend_from_slice(&s.wrapped);
    }
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<Parsed<'_>, CryptoError> {
    let rest = bytes
        .strip_prefix(&MAGIC)
        .ok_or(CryptoError::Malformed("missing magic"))?;
    let (&version, rest) = rest
        .split_first()
        .ok_or(CryptoError::Malformed("truncated header"))?;
    if version != VERSION {
        return Err(CryptoError::UnsupportedVersion(version));
    }
    let (&count, mut rest) = rest
        .split_first()
        .ok_or(CryptoError::Malformed("truncated header"))?;
    let mut stanzas = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let (raw, tail) = rest
            .split_first_chunk::<STANZA_LEN>()
            .ok_or(CryptoError::Malformed("truncated recipient"))?;
        rest = tail;
        let (fingerprint, raw) = split::<FINGERPRINT_LEN>(raw)?;
        let (ephemeral, raw) = split::<KEY_LEN>(raw)?;
        let (nonce, wrapped) = split::<NONCE_LEN>(raw)?;
        stanzas.push(Stanza {
            fingerprint: Fingerprint(*fingerprint),
            ephemeral: *ephemeral,
            nonce: *nonce,
            wrapped: wrapped.to_vec(),
        });
    }
    let (nonce, ciphertext) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(CryptoError::Malformed("truncated nonce"))?;
    Ok(Parsed {
        stanzas,
        nonce,
        ciphertext,
    })
}

fn split<const N: usize>(
    bytes: &[u8],
) -> Result<(&[u8; N], &[u8]), CryptoError> {
    bytes
        .split_first_chunk::<N>()
        .ok_or(CryptoError::Malformed("truncated recipient"))
}
