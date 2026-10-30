// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secret-store backed KEKs: first-run creation, reload, rotation.

use std::collections::HashMap;
use std::sync::Mutex;

use vellum_crypto::{
    CryptoError, KekId, KeyProvider, SecretStore, keyring_from_store,
    load_or_create_kek, open, rewrap, rotate_in_store, seal,
};
use zeroize::Zeroizing;

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, Vec<u8>>>);

impl SecretStore for MemoryStore {
    fn load(
        &self,
        name: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError> {
        let map = self
            .0
            .lock()
            .map_err(|e| CryptoError::Provider(e.to_string()))?;
        Ok(map.get(name).cloned().map(Zeroizing::new))
    }

    fn store(&self, name: &str, secret: &[u8]) -> Result<(), CryptoError> {
        let _previous = self
            .0
            .lock()
            .map_err(|e| CryptoError::Provider(e.to_string()))?
            .insert(name.to_owned(), secret.to_vec());
        Ok(())
    }
}

#[test]
fn creates_once_then_reloads() -> Result<(), CryptoError> {
    let store = MemoryStore::default();
    let id = KekId::new("kek-1")?;
    let first = load_or_create_kek(&store, &id)?;
    let again = load_or_create_kek(&store, &id)?;
    assert_eq!(first.expose(), again.expose());

    store.store("kek-bad", b"short")?;
    assert!(load_or_create_kek(&store, &KekId::new("kek-bad")?).is_err());
    Ok(())
}

#[tokio::test]
async fn rotate_then_restart_reads_old_and_new() -> Result<(), CryptoError> {
    let store = MemoryStore::default();
    let (old, new) = (KekId::new("kek-1")?, KekId::new("kek-2")?);
    let mut ring = keyring_from_store(&store, &old, &[])?;
    let sealed = seal(&ring, b"northstar").await?;

    rotate_in_store(&store, &mut ring, new.clone())?;
    let rewrapped =
        rewrap(&ring, &sealed).await?.ok_or(CryptoError::Decrypt)?;

    let restarted =
        keyring_from_store(&store, &new, std::slice::from_ref(&old))?;
    assert_eq!(restarted.current_kek(), new);
    assert_eq!(open(&restarted, &sealed).await?, b"northstar");
    assert_eq!(open(&restarted, &rewrapped).await?, b"northstar");

    let missing = keyring_from_store(&store, &new, &[KekId::new("kek-0")?]);
    assert!(matches!(missing, Err(CryptoError::UnknownKek(_))));
    Ok(())
}
