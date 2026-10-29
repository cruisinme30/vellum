// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A passphrase KEK can be re-derived from nothing but an envelope header.

use vellum_crypto::{CryptoError, LocalKeyring, PassphraseKdf, kek_of, open, seal};

#[tokio::test]
async fn passphrase_kek_reopens_from_header_alone() -> Result<(), CryptoError> {
    let kdf = PassphraseKdf::with_params(64, 1, 1)?;
    let ring = LocalKeyring::new(kdf.kek_id()?, kdf.derive(b"northstar")?);
    let sealed = seal(&ring, b"vault").await?;

    let id = kek_of(&sealed)?;
    let restored = PassphraseKdf::from_kek_id(&id)?;
    let reopened = LocalKeyring::new(id.clone(), restored.derive(b"northstar")?);
    assert_eq!(open(&reopened, &sealed).await?, b"vault");

    let wrong = LocalKeyring::new(id, restored.derive(b"southstar")?);
    assert!(matches!(
        open(&wrong, &sealed).await,
        Err(CryptoError::Decrypt)
    ));
    Ok(())
}
