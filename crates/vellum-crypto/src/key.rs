// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Key material: 256-bit secret keys and key-encryption key identifiers.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::CryptoError;

/// Length in bytes of every symmetric key (DEK and KEK).
pub const KEY_LEN: usize = 32;

/// Longest accepted [`KekId`], so it fits the envelope's one-byte length field.
pub const MAX_KEK_ID_LEN: usize = 255;

/// A 256-bit symmetric key. The bytes are wiped from memory on drop and never
/// printed by `Debug`.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; KEY_LEN]);

impl SecretKey {
    /// Generates a fresh key from the operating system CSPRNG.
    ///
    /// # Errors
    /// [`CryptoError::Rng`] if the system random source fails.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = [0_u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|_| CryptoError::Rng)?;
        Ok(Self(bytes))
    }

    /// Wraps existing key bytes, for example a KEK read from the OS keychain.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Builds a key from a slice, as returned by a key provider.
    ///
    /// # Errors
    /// [`CryptoError::Malformed`] unless the slice is exactly [`KEY_LEN`] bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, CryptoError> {
        let array: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| CryptoError::Malformed("key has the wrong length"))?;
        Ok(Self(array))
    }

    /// The raw key bytes. Keep the borrow short; never log or persist it.
    #[must_use]
    pub const fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// Names one key-encryption key. Stored in clear in every envelope so the
/// right KEK can be found after rotation. 1–255 bytes of printable ASCII.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KekId(String);

impl KekId {
    /// Validates and wraps an id.
    ///
    /// # Errors
    /// [`CryptoError::InvalidKekId`] if the id is empty, longer than
    /// [`MAX_KEK_ID_LEN`] bytes, or contains anything but printable ASCII.
    pub fn new(id: impl Into<String>) -> Result<Self, CryptoError> {
        let id = id.into();
        let printable = id.bytes().all(|b| b.is_ascii_graphic());
        if id.is_empty() || id.len() > MAX_KEK_ID_LEN || !printable {
            return Err(CryptoError::InvalidKekId);
        }
        Ok(Self(id))
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KekId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_differ_and_debug_hides_bytes() -> Result<(), CryptoError> {
        let a = SecretKey::generate()?;
        let b = SecretKey::generate()?;
        assert_ne!(a.expose(), b.expose());
        assert_eq!(format!("{a:?}"), "SecretKey(..)");
        Ok(())
    }

    #[test]
    fn kek_id_validation() {
        assert!(KekId::new("kek-2026-10").is_ok());
        assert!(KekId::new("").is_err());
        assert!(KekId::new("has space").is_err());
        assert!(KekId::new("é").is_err());
        assert!(KekId::new("a".repeat(MAX_KEK_ID_LEN)).is_ok());
        assert!(KekId::new("a".repeat(MAX_KEK_ID_LEN + 1)).is_err());
    }

    #[test]
    fn from_slice_checks_length() {
        assert!(SecretKey::from_slice(&[0; KEY_LEN]).is_ok());
        assert!(SecretKey::from_slice(&[0; KEY_LEN - 1]).is_err());
    }
}
