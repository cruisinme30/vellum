// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sealed store behaves like a plain store and keeps bytes on disk
//! encrypted, except in plaintext spaces.

use std::sync::Arc;

use bytes::Bytes;
use vellum_crypto::{KekId, LocalKeyring, SecretKey, is_envelope, kek_of};
use vellum_sealed_store::{PrefixPolicy, Sealed};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition, conformance};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn keyring(id: &str) -> Result<LocalKeyring, Box<dyn std::error::Error>> {
    Ok(LocalKeyring::new(KekId::new(id)?, SecretKey::generate()?))
}

async fn sealed(
    dir: &std::path::Path,
    keys: LocalKeyring,
    policy: PrefixPolicy,
) -> Result<Sealed<LocalStore, LocalKeyring, PrefixPolicy>, Box<dyn std::error::Error>> {
    Ok(Sealed::new(
        LocalStore::open(dir).await?,
        Arc::new(keys),
        policy,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn sealed_store_conforms() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = sealed(dir.path(), keyring("kek-1")?, PrefixPolicy::encrypt_all()).await?;
    conformance::run_all(&store).await?;

    let mut plain = PrefixPolicy::encrypt_all();
    plain.set(BlobPath::new("conformance")?, true);
    let dir = tempfile::tempdir()?;
    conformance::run_all(&sealed(dir.path(), keyring("kek-1")?, plain).await?).await?;
    Ok(())
}

#[tokio::test]
async fn disk_holds_ciphertext_except_in_plaintext_spaces() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut policy = PrefixPolicy::encrypt_all();
    policy.set(BlobPath::new("public")?, true);
    let store = sealed(dir.path(), keyring("kek-1")?, policy).await?;
    let doc = Bytes::from_static(b"---\ntitle: Northstar plan\nvellum: 1\n---\n# Plan\n");

    for rel in ["eng/plan.md", "public/plan.md"] {
        let meta = store
            .put(&BlobPath::new(rel)?, doc.clone(), Precondition::None)
            .await?;
        assert_eq!(meta.size, doc.len() as u64);
        assert_eq!(store.get(&BlobPath::new(rel)?).await?.bytes, doc);
        assert_eq!(
            store.head(&BlobPath::new(rel)?).await?.size,
            doc.len() as u64
        );
    }
    let on_disk = std::fs::read(dir.path().join("eng/plan.md"))?;
    assert!(is_envelope(&on_disk));
    assert!(!on_disk.windows(8).any(|w| w == b"Northsta"));
    assert_eq!(std::fs::read(dir.path().join("public/plan.md"))?, doc);
    Ok(())
}

#[tokio::test]
async fn reseal_migrates_a_space_between_modes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = BlobPath::new("space/a.md")?;
    let on_disk = || std::fs::read(dir.path().join("space/a.md"));
    let doc = Bytes::from_static(b"# Northstar");
    let kek = SecretKey::generate()?;
    let ring = || -> Result<LocalKeyring, Box<dyn std::error::Error>> {
        Ok(LocalKeyring::new(KekId::new("kek-1")?, kek.clone()))
    };
    let mut plaintext = PrefixPolicy::encrypt_all();
    plaintext.set(BlobPath::new("space")?, true);

    // Written while the space was plaintext.
    let store = sealed(dir.path(), ring()?, plaintext.clone()).await?;
    let _ = store.put(&path, doc.clone(), Precondition::None).await?;
    assert_eq!(on_disk()?, doc);

    // Switched to encrypted: still readable before and after the reseal.
    let store = sealed(dir.path(), ring()?, PrefixPolicy::encrypt_all()).await?;
    assert_eq!(store.get(&path).await?.bytes, doc);
    assert!(store.reseal(&path).await?);
    assert!(!store.reseal(&path).await?, "already in the wanted form");
    assert!(is_envelope(&on_disk()?));
    assert_eq!(store.get(&path).await?.bytes, doc);

    // Switched back to plaintext.
    let store = sealed(dir.path(), ring()?, plaintext).await?;
    assert_eq!(store.get(&path).await?.bytes, doc);
    assert!(store.reseal(&path).await?);
    assert_eq!(on_disk()?, doc);
    Ok(())
}

#[tokio::test]
async fn rotation_rewraps_on_reseal() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = BlobPath::new("eng/spec.md")?;
    let kek1 = SecretKey::generate()?;
    let ring = LocalKeyring::new(KekId::new("kek-1")?, kek1.clone());
    let store = sealed(dir.path(), ring, PrefixPolicy::encrypt_all()).await?;
    let _ = store
        .put(&path, Bytes::from_static(b"spec"), Precondition::None)
        .await?;

    let mut ring = LocalKeyring::new(KekId::new("kek-1")?, kek1);
    ring.rotate_to(KekId::new("kek-2")?, SecretKey::generate()?);
    let store = sealed(dir.path(), ring, PrefixPolicy::encrypt_all()).await?;
    assert!(store.reseal(&path).await?);
    let on_disk = std::fs::read(dir.path().join("eng/spec.md"))?;
    assert_eq!(kek_of(&on_disk)?.as_str(), "kek-2");
    assert_eq!(store.get(&path).await?.bytes.as_ref(), b"spec");
    Ok(())
}
