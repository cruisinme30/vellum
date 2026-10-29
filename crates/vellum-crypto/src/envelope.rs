// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The on-disk envelope: one fresh DEK per stored object, wrapped by a KEK.
//!
//! Layout (all lengths big-endian):
//!
//! | bytes | field                                     |
//! |-------|-------------------------------------------|
//! | 4     | magic `\0VLE` (a Markdown file never starts with NUL) |
//! | 1     | format version, `1`                       |
//! | 1     | KEK id length `n`                         |
//! | n     | KEK id (printable ASCII)                  |
//! | 2     | wrapped DEK length `w`                    |
//! | w     | wrapped DEK (provider-defined)            |
//! | 24    | payload nonce                             |
//! | rest  | payload ciphertext + 16-byte tag          |
//!
//! The payload is not bound to the object's path, so renames keep working, and
//! not to the header, so rotation re-wraps the DEK without touching the payload.

use crate::aead::{self, NONCE_LEN};
use crate::error::CryptoError;
use crate::key::{KekId, SecretKey};
use crate::provider::{KeyProvider, WrappedDek};

/// First bytes of every envelope.
pub const MAGIC: [u8; 4] = *b"\0VLE";
/// Current envelope format version.
pub const VERSION: u8 = 1;
const PAYLOAD_AAD: &[u8] = b"vellum/blob/v1";

/// True if `bytes` starts like an envelope (plaintext objects never do).
#[must_use]
pub fn is_envelope(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

/// Encrypts `plaintext` under a fresh DEK wrapped by the provider's current KEK.
///
/// # Errors
/// Any provider failure, or [`CryptoError::Rng`]/[`CryptoError::Encrypt`].
#[tracing::instrument(name = "crypto.seal", skip_all, fields(bytes = plaintext.len()))]
pub async fn seal<K: KeyProvider>(provider: &K, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let result = async {
        let dek = SecretKey::generate()?;
        let wrapped = provider.wrap(&dek).await?;
        let (nonce, ciphertext) = aead::seal(&dek, PAYLOAD_AAD, plaintext)?;
        encode(&wrapped, &nonce, &ciphertext)
    }
    .await;
    record("seal", result)
}

/// Decrypts an envelope written by [`seal`] or [`rewrap`].
///
/// # Errors
/// [`CryptoError::Malformed`]/[`CryptoError::UnsupportedVersion`] for bad
/// bytes, [`CryptoError::UnknownKek`] if the KEK is gone, and
/// [`CryptoError::Decrypt`] if anything was tampered with.
#[tracing::instrument(name = "crypto.open", skip_all, fields(bytes = sealed.len()))]
pub async fn open<K: KeyProvider>(provider: &K, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let result = async {
        let parsed = decode(sealed)?;
        let dek = provider.unwrap(&parsed.wrapped).await?;
        aead::open(&dek, parsed.nonce, PAYLOAD_AAD, parsed.ciphertext)
    }
    .await;
    record("open", result)
}

/// Re-wraps the envelope's DEK under the provider's current KEK, leaving the
/// payload bytes untouched. Returns `None` if it is already current.
///
/// # Errors
/// As [`open`], for the DEK only.
#[tracing::instrument(name = "crypto.rewrap", skip_all, fields(bytes = sealed.len()))]
pub async fn rewrap<K: KeyProvider>(
    provider: &K,
    sealed: &[u8],
) -> Result<Option<Vec<u8>>, CryptoError> {
    let result = async {
        let parsed = decode(sealed)?;
        if parsed.wrapped.kek_id == provider.current_kek() {
            return Ok(None);
        }
        let dek = provider.unwrap(&parsed.wrapped).await?;
        let wrapped = provider.wrap(&dek).await?;
        encode(&wrapped, parsed.nonce, parsed.ciphertext).map(Some)
    }
    .await;
    record("rewrap", result)
}

/// Counts every envelope operation by kind and outcome (`OTel` counter
/// `vellum_crypto_ops` via the tracing bridge).
fn record<T>(op: &'static str, result: Result<T, CryptoError>) -> Result<T, CryptoError> {
    let outcome = if result.is_ok() { "ok" } else { "error" };
    tracing::info!(monotonic_counter.vellum_crypto_ops = 1_u64, op, outcome);
    if let Err(error) = &result {
        tracing::warn!(op, %error, "envelope operation failed");
    }
    result
}

/// The KEK an envelope's DEK is wrapped under, read from the header only.
///
/// # Errors
/// [`CryptoError::Malformed`]/[`CryptoError::UnsupportedVersion`] for bad bytes.
pub fn kek_of(sealed: &[u8]) -> Result<KekId, CryptoError> {
    decode(sealed).map(|parsed| parsed.wrapped.kek_id)
}

struct Parsed<'a> {
    wrapped: WrappedDek,
    nonce: &'a [u8; NONCE_LEN],
    ciphertext: &'a [u8],
}

fn encode(
    wrapped: &WrappedDek,
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let id = wrapped.kek_id.as_str().as_bytes();
    let id_len = u8::try_from(id.len()).map_err(|_| CryptoError::InvalidKekId)?;
    let blob_len = u16::try_from(wrapped.blob.len())
        .map_err(|_| CryptoError::Provider("wrapped key longer than 64 KiB".into()))?;
    let mut out =
        Vec::with_capacity(8 + id.len() + wrapped.blob.len() + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(id_len);
    out.extend_from_slice(id);
    out.extend_from_slice(&blob_len.to_be_bytes());
    out.extend_from_slice(&wrapped.blob);
    out.extend_from_slice(nonce);
    out.extend_from_slice(ciphertext);
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<Parsed<'_>, CryptoError> {
    let short = CryptoError::Malformed("truncated header");
    let rest = bytes
        .strip_prefix(&MAGIC)
        .ok_or(CryptoError::Malformed("missing magic"))?;
    let (&version, rest) = rest.split_first().ok_or(short)?;
    if version != VERSION {
        return Err(CryptoError::UnsupportedVersion(version));
    }
    let (&id_len, rest) = rest
        .split_first()
        .ok_or(CryptoError::Malformed("truncated header"))?;
    let (id, rest) = rest
        .split_at_checked(usize::from(id_len))
        .ok_or(CryptoError::Malformed("truncated key id"))?;
    let id = std::str::from_utf8(id).map_err(|_| CryptoError::InvalidKekId)?;
    let kek_id = KekId::new(id)?;
    let (blob_len, rest) = rest
        .split_first_chunk::<2>()
        .ok_or(CryptoError::Malformed("truncated header"))?;
    let (blob, rest) = rest
        .split_at_checked(usize::from(u16::from_be_bytes(*blob_len)))
        .ok_or(CryptoError::Malformed("truncated wrapped key"))?;
    let (nonce, ciphertext) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(CryptoError::Malformed("truncated nonce"))?;
    Ok(Parsed {
        wrapped: WrappedDek {
            kek_id,
            blob: blob.to_vec(),
        },
        nonce,
        ciphertext,
    })
}
