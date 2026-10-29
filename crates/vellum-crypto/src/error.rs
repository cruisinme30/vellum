// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Error type for every fallible operation in this crate.

use crate::key::KekId;

/// Everything that can go wrong while sealing, opening or re-wrapping data.
///
/// Messages never contain key material or plaintext.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// The operating system random number generator failed.
    #[error("system random number generator failed")]
    Rng,
    /// The AEAD refused to encrypt (input larger than the cipher allows).
    #[error("encryption failed")]
    Encrypt,
    /// Authentication failed: wrong key, or the data was tampered with.
    #[error("decryption failed: wrong key or tampered data")]
    Decrypt,
    /// The envelope names a key-encryption key the provider does not hold.
    #[error("unknown key-encryption key `{0}`")]
    UnknownKek(KekId),
    /// The bytes are not a well-formed envelope.
    #[error("malformed envelope: {0}")]
    Malformed(&'static str),
    /// The envelope was written by a newer format version.
    #[error("unsupported envelope version {0}")]
    UnsupportedVersion(u8),
    /// A key-encryption key id is empty, too long or not printable ASCII.
    #[error("invalid key-encryption key id")]
    InvalidKekId,
    /// Passphrase key derivation failed or its parameters are invalid.
    #[error("passphrase key derivation failed")]
    Kdf,
    /// A recipient (E2E) envelope operation was refused.
    #[error("recipient envelope: {0}")]
    Recipients(&'static str),
    /// A remote or OS key provider (keychain, KMS, Vault) failed.
    #[error("key provider unavailable: {0}")]
    Provider(String),
}
