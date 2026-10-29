// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The device's own key pair.

use std::fmt;

use vellum_crypto::recipients::{self, Fingerprint, RecipientPublic, RecipientSecret};
use vellum_crypto::{CryptoError, SecretStore};

/// The name the device secret is stored under in the secret store.
pub const DEVICE_KEY_NAME: &str = "vellum-device-x25519-v1";

/// Length of an X25519 key.
const KEY_LEN: usize = 32;

/// This device's X25519 key pair. The secret never leaves the secret store
/// and this process; only the public key is shared, for enrolment.
pub struct DeviceIdentity {
    secret: RecipientSecret,
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("fingerprint", &self.fingerprint_hex())
            .finish_non_exhaustive()
    }
}

impl DeviceIdentity {
    /// Loads the device key from `store`, creating and saving one on first
    /// use.
    ///
    /// # Errors
    ///
    /// The store cannot be reached, the stored value is not a 32-byte key,
    /// or the random source fails.
    #[tracing::instrument(name = "private.identity.load", skip_all)]
    pub fn load_or_create(store: &dyn SecretStore) -> Result<Self, CryptoError> {
        if let Some(bytes) = store.load(DEVICE_KEY_NAME)? {
            let raw: [u8; KEY_LEN] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| CryptoError::Malformed("the stored device key is not 32 bytes"))?;
            return Ok(Self::from_secret(RecipientSecret::from_bytes(raw)));
        }
        let secret = RecipientSecret::generate()?;
        store.store(DEVICE_KEY_NAME, secret.expose())?;
        tracing::info!("created a new device key");
        Ok(Self::from_secret(secret))
    }

    /// Wraps a key the caller already holds.
    #[must_use]
    pub const fn from_secret(secret: RecipientSecret) -> Self {
        Self { secret }
    }

    /// The public key, for the space's member registry.
    #[must_use]
    pub fn public(&self) -> RecipientPublic {
        self.secret.public()
    }

    /// The short id envelopes use for this device.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        self.public().fingerprint()
    }

    /// The fingerprint as lowercase hex, for display.
    #[must_use]
    pub fn fingerprint_hex(&self) -> String {
        hex(&self.fingerprint().to_bytes())
    }

    /// The public key as lowercase hex, for display and enrolment.
    #[must_use]
    pub fn public_hex(&self) -> String {
        hex(&self.public().to_bytes())
    }

    /// Opens an envelope sealed to this device.
    ///
    /// # Errors
    ///
    /// [`CryptoError::Decrypt`] when the envelope has no stanza for this
    /// device or was tampered with; other variants for malformed input.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
        recipients::open_as(&self.secret, sealed)
    }
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use vellum_crypto::recipients::seal_for;
    use zeroize::Zeroizing;

    use super::*;

    #[derive(Default)]
    struct Memory(Mutex<HashMap<String, Vec<u8>>>);

    impl SecretStore for Memory {
        fn load(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .map(Zeroizing::new))
        }

        fn store(&self, name: &str, secret: &[u8]) -> Result<(), CryptoError> {
            let _ = self
                .0
                .lock()
                .unwrap()
                .insert(name.to_owned(), secret.to_vec());
            Ok(())
        }
    }

    #[test]
    fn the_key_is_created_once_and_then_reused() {
        let store = Memory::default();
        let first = DeviceIdentity::load_or_create(&store).unwrap();
        let again = DeviceIdentity::load_or_create(&store).unwrap();
        assert_eq!(first.public(), again.public());
        assert_eq!(first.fingerprint_hex().len(), 32);
        assert_eq!(first.public_hex().len(), 64);
        assert!(!format!("{first:?}").contains(&first.public_hex()));
    }

    #[test]
    fn a_damaged_stored_key_is_refused() {
        let store = Memory::default();
        store.store(DEVICE_KEY_NAME, &[1, 2, 3]).unwrap();
        assert!(matches!(
            DeviceIdentity::load_or_create(&store),
            Err(CryptoError::Malformed(_))
        ));
    }

    #[test]
    fn it_opens_what_is_sealed_to_it_and_nothing_else() {
        let me = DeviceIdentity::from_secret(RecipientSecret::generate().unwrap());
        let other = DeviceIdentity::from_secret(RecipientSecret::generate().unwrap());
        let sealed = seal_for(&[me.public()], b"hello").unwrap();
        assert_eq!(me.open(&sealed).unwrap(), b"hello");
        assert!(other.open(&sealed).is_err());
    }
}
