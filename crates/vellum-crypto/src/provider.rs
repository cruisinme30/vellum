// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Key providers: who holds the key-encryption keys (KEKs).
//!
//! A [`KeyProvider`] wraps and unwraps data-encryption keys. The KEK itself may
//! never leave the provider, which is what lets AWS/GCP KMS and Vault transit
//! implement the same trait as the local providers here: the wrapped form is
//! opaque bytes the provider defines.

use std::collections::BTreeMap;
use std::future::Future;

use crate::aead::{self, NONCE_LEN};
use crate::error::CryptoError;
use crate::key::{KekId, SecretKey};

/// Associated data bound into every locally wrapped DEK.
const WRAP_AAD_PREFIX: &[u8] = b"vellum/dek/v1:";

/// A data-encryption key sealed under one KEK. `blob` is provider-defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedDek {
    /// The KEK the DEK is wrapped under.
    pub kek_id: KekId,
    /// Provider-defined ciphertext of the DEK.
    pub blob: Vec<u8>,
}

/// Holds KEKs and wraps/unwraps DEKs with them.
///
/// Implementations: [`LocalKeyring`] (keys in memory, fed from the OS keychain
/// or a passphrase), and remote adapters (KMS, Vault) that call out over the
/// network, which is why the methods are async.
pub trait KeyProvider: Send + Sync {
    /// The KEK that new DEKs are wrapped under.
    fn current_kek(&self) -> KekId;

    /// Wraps `dek` under the current KEK.
    fn wrap(&self, dek: &SecretKey)
    -> impl Future<Output = Result<WrappedDek, CryptoError>> + Send;

    /// Unwraps a DEK under whichever KEK `wrapped.kek_id` names.
    fn unwrap(
        &self,
        wrapped: &WrappedDek,
    ) -> impl Future<Output = Result<SecretKey, CryptoError>> + Send;
}

/// KEKs held in process memory: one current KEK for new wraps, plus retired
/// KEKs kept only to unwrap old envelopes until rotation has re-wrapped them.
#[derive(Debug)]
pub struct LocalKeyring {
    current: KekId,
    keks: BTreeMap<KekId, SecretKey>,
}

impl LocalKeyring {
    /// A keyring whose only (and current) KEK is `kek`.
    #[must_use]
    pub fn new(id: KekId, kek: SecretKey) -> Self {
        let keks = BTreeMap::from([(id.clone(), kek)]);
        Self { current: id, keks }
    }

    /// Adds a KEK used only to unwrap existing envelopes.
    pub fn add_retired(&mut self, id: KekId, kek: SecretKey) {
        let _ = self.keks.insert(id, kek);
    }

    /// Makes `kek` current. The previous KEK stays available for unwrapping.
    pub fn rotate_to(&mut self, id: KekId, kek: SecretKey) {
        let _ = self.keks.insert(id.clone(), kek);
        self.current = id;
    }

    /// Forgets a retired KEK once nothing is wrapped under it any more.
    /// Returns `false` (and keeps it) if `id` is the current KEK.
    pub fn forget(&mut self, id: &KekId) -> bool {
        if *id == self.current {
            return false;
        }
        self.keks.remove(id).is_some()
    }

    /// Ids of every KEK held, current included.
    pub fn kek_ids(&self) -> impl Iterator<Item = &KekId> {
        self.keks.keys()
    }

    fn kek(&self, id: &KekId) -> Result<&SecretKey, CryptoError> {
        self.keks
            .get(id)
            .ok_or_else(|| CryptoError::UnknownKek(id.clone()))
    }

    fn wrap_now(&self, dek: &SecretKey) -> Result<WrappedDek, CryptoError> {
        let kek = self.kek(&self.current)?;
        let (nonce, ciphertext) = aead::seal(kek, &wrap_aad(&self.current), dek.expose())?;
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(WrappedDek {
            kek_id: self.current.clone(),
            blob,
        })
    }

    fn unwrap_now(&self, wrapped: &WrappedDek) -> Result<SecretKey, CryptoError> {
        let kek = self.kek(&wrapped.kek_id)?;
        let (nonce, ciphertext) = wrapped
            .blob
            .split_first_chunk::<NONCE_LEN>()
            .ok_or(CryptoError::Malformed("wrapped key too short"))?;
        let plain = zeroize::Zeroizing::new(aead::open(
            kek,
            nonce,
            &wrap_aad(&wrapped.kek_id),
            ciphertext,
        )?);
        SecretKey::from_slice(&plain)
    }
}

impl KeyProvider for LocalKeyring {
    fn current_kek(&self) -> KekId {
        self.current.clone()
    }

    fn wrap(
        &self,
        dek: &SecretKey,
    ) -> impl Future<Output = Result<WrappedDek, CryptoError>> + Send {
        std::future::ready(self.wrap_now(dek))
    }

    fn unwrap(
        &self,
        wrapped: &WrappedDek,
    ) -> impl Future<Output = Result<SecretKey, CryptoError>> + Send {
        std::future::ready(self.unwrap_now(wrapped))
    }
}

fn wrap_aad(id: &KekId) -> Vec<u8> {
    [WRAP_AAD_PREFIX, id.as_str().as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wraps_under_current_and_unwraps_retired() -> Result<(), CryptoError> {
        let old = KekId::new("kek-old")?;
        let mut ring = LocalKeyring::new(old.clone(), SecretKey::generate()?);
        let dek = SecretKey::generate()?;
        let wrapped = ring.wrap(&dek).await?;
        assert_eq!(wrapped.kek_id, old);

        ring.rotate_to(KekId::new("kek-new")?, SecretKey::generate()?);
        assert_eq!(ring.current_kek().as_str(), "kek-new");
        assert_eq!(ring.unwrap(&wrapped).await?.expose(), dek.expose());
        assert_eq!(ring.kek_ids().count(), 2);

        assert!(ring.forget(&old));
        assert!(matches!(
            ring.unwrap(&wrapped).await,
            Err(CryptoError::UnknownKek(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn wrapped_key_is_bound_to_its_kek_id() -> Result<(), CryptoError> {
        let kek = SecretKey::from_bytes([7; 32]);
        let ring = LocalKeyring::new(KekId::new("a")?, kek.clone());
        let mut wrapped = ring.wrap(&SecretKey::generate()?).await?;
        let mut other = LocalKeyring::new(KekId::new("b")?, kek);
        other.add_retired(KekId::new("a")?, SecretKey::from_bytes([7; 32]));
        wrapped.kek_id = KekId::new("b")?;
        assert!(matches!(
            other.unwrap(&wrapped).await,
            Err(CryptoError::Decrypt)
        ));
        wrapped.blob.truncate(10);
        assert!(other.unwrap(&wrapped).await.is_err());
        Ok(())
    }
}
