// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! KEKs persisted in a secret store, by default the OS keychain.
//!
//! The KEK's raw bytes live in the store under its [`KekId`]; the server
//! config records which id is current and which are retired. The OS backend
//! (macOS Keychain, Windows Credential Manager, the Linux Secret Service) is
//! behind the `os-keychain` feature. The Secret Service needs a running
//! keyring daemon (GNOME Keyring or KDE Wallet), which desktops have and headless
//! Linux servers usually lack; those use a passphrase or a KMS/Vault provider.

use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::key::{KekId, SecretKey};
use crate::provider::LocalKeyring;

/// Somewhere KEK bytes persist between runs, keyed by KEK id.
pub trait SecretStore: Send + Sync {
    /// Reads the secret stored under `name`, or `None` if there is none.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the store cannot be reached.
    fn load(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError>;

    /// Stores `secret` under `name`, replacing any previous value.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the store cannot be reached.
    fn store(&self, name: &str, secret: &[u8]) -> Result<(), CryptoError>;
}

/// Loads the KEK stored under `id`, generating and saving one on first run.
///
/// # Errors
/// Store failures, or [`CryptoError::Malformed`] if the stored value is not a
/// 32-byte key.
#[tracing::instrument(name = "crypto.kek.load_or_create", skip(store), fields(kek_id = %id))]
pub fn load_or_create_kek(store: &impl SecretStore, id: &KekId) -> Result<SecretKey, CryptoError> {
    if let Some(bytes) = store.load(id.as_str())? {
        return SecretKey::from_slice(&bytes);
    }
    let kek = SecretKey::generate()?;
    store.store(id.as_str(), kek.expose())?;
    tracing::info!("generated new key-encryption key");
    Ok(kek)
}

/// Builds a keyring from the store: `current` is loaded or created, every
/// `retired` id must already exist.
///
/// # Errors
/// [`CryptoError::UnknownKek`] if a retired KEK is missing from the store, or
/// any error of [`load_or_create_kek`].
pub fn keyring_from_store(
    store: &impl SecretStore,
    current: &KekId,
    retired: &[KekId],
) -> Result<LocalKeyring, CryptoError> {
    let mut ring = LocalKeyring::new(current.clone(), load_or_create_kek(store, current)?);
    for id in retired {
        let bytes = store
            .load(id.as_str())?
            .ok_or_else(|| CryptoError::UnknownKek(id.clone()))?;
        ring.add_retired(id.clone(), SecretKey::from_slice(&bytes)?);
    }
    Ok(ring)
}

/// Creates a new KEK under `new_id`, saves it, and makes it current in `ring`.
/// Re-wrap existing envelopes afterwards with [`crate::rewrap`].
///
/// # Errors
/// Store or RNG failures. The ring is unchanged on error.
pub fn rotate_in_store(
    store: &impl SecretStore,
    ring: &mut LocalKeyring,
    new_id: KekId,
) -> Result<(), CryptoError> {
    let kek = SecretKey::generate()?;
    store.store(new_id.as_str(), kek.expose())?;
    ring.rotate_to(new_id, kek);
    Ok(())
}

/// The operating system keychain: macOS Keychain, Windows Credential
/// Manager or the Linux Secret Service. Entries use `service` and the KEK id as the account name.
#[cfg(feature = "os-keychain")]
pub struct OsKeychain {
    service: String,
    store: std::sync::Arc<keyring_core::CredentialStore>,
}

#[cfg(feature = "os-keychain")]
impl std::fmt::Debug for OsKeychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsKeychain")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "os-keychain")]
impl OsKeychain {
    /// Opens the platform keychain for `service`, e.g. `dev.vellum.server`.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the keychain cannot be opened, or on a
    /// platform without one, or on Linux without a Secret Service daemon
    /// (headless servers use a passphrase or KMS/Vault).
    pub fn open(service: impl Into<String>) -> Result<Self, CryptoError> {
        Ok(Self {
            service: service.into(),
            store: platform_store()?,
        })
    }

    fn entry(&self, name: &str) -> Result<keyring_core::Entry, CryptoError> {
        self.store
            .build(&self.service, name, None)
            .map_err(|e| provider_error(&e))
    }
}

#[cfg(feature = "os-keychain")]
impl SecretStore for OsKeychain {
    fn load(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError> {
        match self.entry(name)?.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(provider_error(&e)),
        }
    }

    fn store(&self, name: &str, secret: &[u8]) -> Result<(), CryptoError> {
        self.entry(name)?
            .set_secret(secret)
            .map_err(|e| provider_error(&e))
    }
}

#[cfg(feature = "os-keychain")]
fn provider_error(error: &keyring_core::Error) -> CryptoError {
    CryptoError::Provider(error.to_string())
}

#[cfg(all(feature = "os-keychain", target_os = "macos"))]
fn platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, CryptoError> {
    let store =
        apple_native_keyring_store::keychain::Store::new().map_err(|e| provider_error(&e))?;
    Ok(store)
}

#[cfg(all(feature = "os-keychain", windows))]
fn platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, CryptoError> {
    let store = windows_native_keyring_store::Store::new().map_err(|e| provider_error(&e))?;
    Ok(store)
}

#[cfg(all(feature = "os-keychain", target_os = "linux"))]
fn platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, CryptoError> {
    let store = dbus_secret_service_keyring_store::Store::new().map_err(|e| provider_error(&e))?;
    Ok(store)
}

#[cfg(all(
    feature = "os-keychain",
    not(any(target_os = "macos", target_os = "linux", windows))
))]
fn platform_store() -> Result<std::sync::Arc<keyring_core::CredentialStore>, CryptoError> {
    Err(CryptoError::Provider(
        "no OS keychain on this platform; use a passphrase or KMS/Vault KEK".into(),
    ))
}
