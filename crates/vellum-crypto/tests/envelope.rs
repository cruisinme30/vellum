// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Round-trip, tamper, rotation and format tests for the envelope.

use proptest::prelude::*;
use vellum_crypto::{
    CryptoError, KekId, LocalKeyring, SecretKey, is_envelope, kek_of, open, rewrap, seal,
};

type TestResult = Result<(), CryptoError>;

fn keyring(id: &str) -> Result<LocalKeyring, CryptoError> {
    Ok(LocalKeyring::new(KekId::new(id)?, SecretKey::generate()?))
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_or_else(|_| unreachable!(), |rt| rt.block_on(future))
}

#[tokio::test]
async fn round_trip_and_fresh_dek_per_object() -> TestResult {
    let ring = keyring("kek-a")?;
    let doc = b"---\nid: 01JB\ntitle: Northstar roadmap\nvellum: 1\n---\n# Roadmap\n";
    let one = seal(&ring, doc).await?;
    let two = seal(&ring, doc).await?;
    assert!(is_envelope(&one));
    assert!(!is_envelope(doc));
    assert_ne!(one, two, "each object gets its own DEK and nonce");
    assert_eq!(open(&ring, &one).await?, doc);
    assert_eq!(kek_of(&one)?.as_str(), "kek-a");
    Ok(())
}

#[tokio::test]
async fn wrong_kek_and_tampering_fail() -> TestResult {
    let ring = keyring("kek-a")?;
    let other = keyring("kek-a")?;
    let sealed = seal(&ring, b"secret").await?;
    assert!(matches!(
        open(&other, &sealed).await,
        Err(CryptoError::Decrypt)
    ));
    assert!(matches!(
        open(&keyring("kek-b")?, &sealed).await,
        Err(CryptoError::UnknownKek(_))
    ));
    for at in [7, 20, sealed.len() - 1] {
        let mut bad = sealed.clone();
        if let Some(byte) = bad.get_mut(at) {
            *byte ^= 1;
        }
        assert!(open(&ring, &bad).await.is_err(), "flip at {at}");
    }
    Ok(())
}

#[tokio::test]
async fn rotation_rewraps_without_touching_payload() -> TestResult {
    let mut ring = keyring("kek-2026-01")?;
    let sealed = seal(&ring, b"rotate me").await?;
    assert!(rewrap(&ring, &sealed).await?.is_none());

    let new_id = KekId::new("kek-2026-10")?;
    ring.rotate_to(new_id.clone(), SecretKey::generate()?);
    let rewrapped = rewrap(&ring, &sealed).await?.ok_or(CryptoError::Decrypt)?;
    assert_eq!(kek_of(&rewrapped)?, new_id);
    assert!(rewrapped.ends_with(sealed.get(sealed.len() - 40..).unwrap_or_default()));

    assert!(!ring.forget(&new_id), "current KEK cannot be forgotten");
    assert!(ring.forget(&KekId::new("kek-2026-01")?));
    assert_eq!(open(&ring, &rewrapped).await?, b"rotate me");
    assert!(open(&ring, &sealed).await.is_err());
    Ok(())
}

#[test]
fn rejects_malformed_headers() {
    let cases: [&[u8]; 5] = [
        b"",
        b"# plain markdown",
        b"\0VLE\x02",
        b"\0VLE\x01\x05kek",
        b"\0VLE\x01\x01k\x00\x00short-nonce",
    ];
    for case in cases {
        assert!(kek_of(case).is_err(), "{case:?}");
    }
    assert!(matches!(
        kek_of(b"\0VLE\x09"),
        Err(CryptoError::UnsupportedVersion(9))
    ));
}

proptest! {
    #[test]
    fn any_payload_round_trips(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let ring = keyring("kek-prop").map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back = block_on(async { open(&ring, &seal(&ring, &data).await?).await })
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, data);
    }

    #[test]
    fn garbage_never_panics(data in proptest::collection::vec(any::<u8>(), 0..256)) {
        let ring = keyring("kek-prop").map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut framed = b"\0VLE\x01".to_vec();
        framed.extend_from_slice(&data);
        prop_assert!(block_on(open(&ring, &framed)).is_err());
    }
}
