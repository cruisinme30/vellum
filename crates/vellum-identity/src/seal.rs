// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secrets identity must read back, sealed at rest (ADR 0069, D290, D292).
//!
//! Most credentials are stored only as hashes ([`crate::secret`]). A few
//! must be read back: authenticator-app (TOTP) secrets and identity
//! providers' client secrets. They are sealed with AES-256-GCM under the
//! identity key, which the composition root keeps in
//! `.vellum/keys/identity.key` (or `VELLUM_IDENTITY_KEY`), so a copy of
//! the records or the database alone reveals none of them. The associated
//! data names what the secret is for, so a sealed value cannot be moved
//! to another field or account.

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use vellum_kernel::{Error, ErrorKind};

/// Seals and opens secrets under the identity key.
pub struct Sealer {
    key: LessSafeKey,
}

impl std::fmt::Debug for Sealer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sealer(..)")
    }
}

fn internal(what: &str) -> Error {
    Error::new(ErrorKind::Internal, what.to_owned())
}

impl Sealer {
    /// A sealer over a 32-byte key.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the key is refused.
    pub fn new(key: &[u8; 32]) -> Result<Self, Error> {
        let key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| internal("bad identity key"))?;
        Ok(Self {
            key: LessSafeKey::new(key),
        })
    }

    /// A sealer over a random key, for tests and servers without a data
    /// folder: what it seals cannot be opened after a restart.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the random source fails.
    pub fn ephemeral() -> Result<Self, Error> {
        let mut key = [0_u8; 32];
        getrandom::fill(&mut key).map_err(|_| internal("random source failed"))?;
        Self::new(&key)
    }

    /// Seals `plaintext` for `purpose`, as base64url text.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the random source or cipher fails.
    pub fn seal(&self, purpose: &str, plaintext: &[u8]) -> Result<String, Error> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| internal("random source failed"))?;
        let mut sealed = plaintext.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(purpose.as_bytes()),
                &mut sealed,
            )
            .map_err(|_| internal("cannot seal a secret"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        Ok(URL_SAFE_NO_PAD.encode(out))
    }

    /// Opens what [`Sealer::seal`] sealed for the same `purpose`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] for malformed text, another purpose or key,
    /// or tampering.
    pub fn open(&self, purpose: &str, sealed: &str) -> Result<Vec<u8>, Error> {
        let bytes = URL_SAFE_NO_PAD
            .decode(sealed)
            .map_err(|_| internal("a sealed secret is malformed"))?;
        let (nonce, body) = bytes
            .split_at_checked(NONCE_LEN)
            .ok_or_else(|| internal("a sealed secret is too short"))?;
        let nonce = Nonce::try_assume_unique_for_key(nonce)
            .map_err(|_| internal("a sealed secret is malformed"))?;
        let mut body = body.to_vec();
        let opened = self
            .key
            .open_in_place(nonce, Aad::from(purpose.as_bytes()), &mut body)
            .map_err(|_| {
                internal(
                    "a sealed secret does not open: the identity key changed or it was altered",
                )
            })?;
        Ok(opened.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_secrets_open_only_for_their_purpose_and_key() {
        let key = Sealer::new(&[7; 32]).unwrap();
        let sealed = key.seal("totp:u1", b"JBSWY3DPEHPK3PXP").unwrap();
        assert!(!sealed.contains("JBSW"));
        assert_eq!(key.open("totp:u1", &sealed).unwrap(), b"JBSWY3DPEHPK3PXP");
        assert!(key.open("totp:u2", &sealed).is_err());
        assert!(
            Sealer::new(&[8; 32])
                .unwrap()
                .open("totp:u1", &sealed)
                .is_err()
        );
        let mut tampered = sealed.into_bytes();
        let last = tampered.len() - 2;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(key.open("totp:u1", &tampered).is_err());
        assert!(key.open("totp:u1", "short").is_err());
        assert_ne!(key.seal("p", b"x").unwrap(), key.seal("p", b"x").unwrap());
    }
}
