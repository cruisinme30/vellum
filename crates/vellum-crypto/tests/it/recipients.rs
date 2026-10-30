// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Recipient (E2E) envelopes: members open, others cannot, membership
//! changes never touch the payload.

use proptest::prelude::*;
use vellum_crypto::recipients::{
    RecipientPublic, RecipientSecret, add_recipient, is_recipient_envelope, open_as, recipients_of,
    remove_recipient, seal_for,
};
use vellum_crypto::{CryptoError, is_envelope};

type TestResult = Result<(), CryptoError>;

fn hex32(text: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (slot, i) in out.iter_mut().zip((0..64).step_by(2)) {
        *slot = u8::from_str_radix(text.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    }
    out
}

#[test]
fn x25519_matches_rfc_7748() {
    let alice = RecipientSecret::from_bytes(hex32(
        "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a",
    ));
    let bob = RecipientSecret::from_bytes(hex32(
        "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb",
    ));
    assert_eq!(
        alice.public().to_bytes(),
        hex32("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    );
    assert_eq!(
        bob.public().to_bytes(),
        hex32("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
    );
}

#[test]
fn members_open_and_outsiders_cannot() -> TestResult {
    let (ana, ben, eve) = (
        RecipientSecret::generate()?,
        RecipientSecret::generate()?,
        RecipientSecret::generate()?,
    );
    let doc = b"# Northstar board notes";
    let sealed = seal_for(&[ana.public(), ben.public(), ana.public()], doc)?;
    assert!(is_envelope(&sealed) && is_recipient_envelope(&sealed));
    assert_eq!(recipients_of(&sealed)?.len(), 2, "duplicates collapse");
    assert_eq!(open_as(&ana, &sealed)?, doc);
    assert_eq!(open_as(&ben, &sealed)?, doc);
    assert!(matches!(
        open_as(&eve, &sealed),
        Err(CryptoError::Recipients(_))
    ));
    assert!(matches!(
        vellum_crypto::kek_of(&sealed),
        Err(CryptoError::UnsupportedVersion(2))
    ));
    Ok(())
}

#[test]
fn membership_changes_keep_the_payload() -> TestResult {
    let (ana, ben) = (RecipientSecret::generate()?, RecipientSecret::generate()?);
    let sealed = seal_for(&[ana.public()], b"plan")?;
    let tail = sealed.get(sealed.len() - 20..).unwrap_or_default().to_vec();

    assert!(open_as(&ben, &sealed).is_err());
    let added = add_recipient(&ana, &ben.public(), &sealed)?;
    assert_eq!(add_recipient(&ana, &ben.public(), &added)?, added);
    assert_eq!(open_as(&ben, &added)?, b"plan");
    assert!(added.ends_with(&tail));
    assert!(add_recipient(&RecipientSecret::generate()?, &ben.public(), &sealed).is_err());

    let removed = remove_recipient(&ana.public(), &added)?;
    assert!(open_as(&ana, &removed).is_err());
    assert_eq!(open_as(&ben, &removed)?, b"plan");
    assert!(matches!(
        remove_recipient(&ben.public(), &removed),
        Err(CryptoError::Recipients(_))
    ));
    Ok(())
}

#[test]
fn rejects_bad_inputs() -> TestResult {
    let ana = RecipientSecret::generate()?;
    assert!(seal_for(&[], b"x").is_err());
    assert!(seal_for(&[RecipientPublic::from_bytes([0; 32])], b"x").is_err());
    let many: Vec<_> = (0..256)
        .map(|_| RecipientSecret::generate().map(|s| s.public()))
        .collect::<Result<_, _>>()?;
    assert!(seal_for(&many, b"x").is_err());

    let sealed = seal_for(&[ana.public()], b"secret")?;
    for at in [6, 30, 100, sealed.len() - 1] {
        let mut bad = sealed.clone();
        if let Some(byte) = bad.get_mut(at) {
            *byte ^= 1;
        }
        assert!(open_as(&ana, &bad).is_err(), "flip at {at}");
    }
    assert!(open_as(&ana, sealed.get(..50).unwrap_or_default()).is_err());
    assert_eq!(format!("{ana:?}"), "RecipientSecret(..)");
    Ok(())
}

proptest! {
    #[test]
    fn garbage_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let ana = RecipientSecret::generate().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut framed = b"\0VLE\x02".to_vec();
        framed.extend_from_slice(&data);
        prop_assert!(open_as(&ana, &framed).is_err());
        let _ = recipients_of(&framed);
    }
}
