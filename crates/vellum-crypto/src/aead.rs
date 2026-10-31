// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! XChaCha20-Poly1305 with random 192-bit nonces.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::CryptoError;
use crate::key::SecretKey;

/// Nonce length of XChaCha20-Poly1305. Large enough to draw at random.
pub const NONCE_LEN: usize = 24;

/// Poly1305 authentication tag length appended to every ciphertext.
pub const TAG_LEN: usize = 16;

/// Encrypts `plaintext` under `key`, returning a fresh random nonce and the
/// ciphertext (with tag appended).
///
/// # Errors
/// [`CryptoError::Rng`] or [`CryptoError::Encrypt`].
pub fn seal(
    key: &SecretKey,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<([u8; NONCE_LEN], Vec<u8>), CryptoError> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| CryptoError::Rng)?;
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    let ciphertext = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;
    Ok((nonce, ciphertext))
}

/// Decrypts and authenticates `ciphertext`.
///
/// # Errors
/// [`CryptoError::Decrypt`] on a wrong key, nonce or AAD, or tampering.
pub fn open(
    key: &SecretKey,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    cipher
        .decrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_aad_binding() -> Result<(), CryptoError> {
        let key = SecretKey::generate()?;
        let (nonce, ciphertext) = seal(&key, b"aad", b"northstar")?;
        assert_eq!(ciphertext.len(), b"northstar".len() + TAG_LEN);
        assert_eq!(open(&key, &nonce, b"aad", &ciphertext)?, b"northstar");
        assert!(open(&key, &nonce, b"other", &ciphertext).is_err());
        assert!(
            open(&SecretKey::generate()?, &nonce, b"aad", &ciphertext).is_err()
        );
        Ok(())
    }
}
