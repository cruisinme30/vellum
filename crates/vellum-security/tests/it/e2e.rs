// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! E2E private spaces: key registry and server-feature policy.

#![allow(clippy::unwrap_used)]

use vellum_kernel::{ErrorKind, SpaceId, UserId};
use vellum_security::e2e::{
    DeviceKey, E2eError, KeyRegistry, MAX_RECIPIENTS, ServerFeature, SpaceMode,
};

fn key(n: u8) -> DeviceKey {
    let mut bytes = [0x42; 32];
    bytes[0] = n;
    bytes[1] = n.wrapping_mul(31);
    DeviceKey::new(bytes).unwrap()
}

#[test]
fn small_order_keys_are_refused() {
    let mut one = [0; 32];
    one[0] = 1;
    let mut p_minus_1 = [0xff; 32];
    p_minus_1[0] = 0xec;
    p_minus_1[31] = 0x7f;
    let mut p_minus_1_high = p_minus_1;
    p_minus_1_high[31] = 0xff;
    for weak in [[0; 32], one, p_minus_1, p_minus_1_high] {
        assert_eq!(DeviceKey::new(weak), Err(E2eError::WeakKey));
    }
    assert!(DeviceKey::new([9; 32]).is_ok());
}

#[test]
fn fingerprint_matches_the_envelope_stanza_id() {
    use sha2::Digest;
    let key = DeviceKey::new([9; 32]).unwrap();
    let digest = sha2::Sha256::digest([9_u8; 32]);
    assert_eq!(key.fingerprint()[..], digest[..16]);
}

#[test]
fn recipients_cover_every_member_device() {
    let space = SpaceId::generate();
    let (ana, ben) = (UserId::generate(), UserId::generate());
    let mut reg = KeyRegistry::new();
    reg.enroll(space, ana, key(1)).unwrap();
    reg.enroll(space, ana, key(2)).unwrap();
    reg.enroll(space, ben, key(3)).unwrap();
    let mut all = reg.recipients(space);
    all.sort();
    let mut want = vec![key(1), key(2), key(3)];
    want.sort();
    assert_eq!(all, want);
    assert_eq!(reg.keys_of(space, ana), [key(1), key(2)]);
    assert!(reg.recipients(SpaceId::generate()).is_empty());
    assert_eq!(reg.enroll(space, ben, key(1)), Err(E2eError::Duplicate));
}

#[test]
fn removals_report_keys_to_strip() {
    let space = SpaceId::generate();
    let (ana, ben) = (UserId::generate(), UserId::generate());
    let mut reg = KeyRegistry::new();
    reg.enroll(space, ana, key(1)).unwrap();
    reg.enroll(space, ben, key(2)).unwrap();
    reg.enroll(space, ben, key(3)).unwrap();

    let removed = reg.revoke_device(space, ben, key(2)).unwrap();
    assert_eq!(removed.keys, [key(2)]);
    assert_eq!(
        reg.revoke_device(space, ben, key(2)),
        Err(E2eError::NotEnrolled)
    );

    let removed = reg.remove_member(space, ben).unwrap();
    assert_eq!((removed.space, removed.keys), (space, vec![key(3)]));
    assert_eq!(reg.recipients(space), [key(1)]);

    // The last key can never go: nobody could decrypt the space again.
    assert_eq!(reg.remove_member(space, ana), Err(E2eError::LastKey));
    assert_eq!(
        reg.revoke_device(space, ana, key(1)),
        Err(E2eError::LastKey)
    );
}

#[test]
fn key_count_is_capped() {
    let space = SpaceId::generate();
    let mut reg = KeyRegistry::new();
    for n in 0..MAX_RECIPIENTS {
        let mut bytes = [7; 32];
        bytes[..8].copy_from_slice(&(n as u64).to_le_bytes());
        reg.enroll(space, UserId::generate(), DeviceKey::new(bytes).unwrap())
            .unwrap();
    }
    assert_eq!(
        reg.enroll(space, UserId::generate(), key(200)),
        Err(E2eError::TooManyKeys)
    );
}

#[test]
fn e2e_spaces_refuse_server_features() {
    for feature in [
        ServerFeature::Search,
        ServerFeature::Render,
        ServerFeature::Mcp,
        ServerFeature::Export,
        ServerFeature::PublicShare,
        ServerFeature::Unfurl,
        ServerFeature::SecretScan,
        ServerFeature::PlaintextSync,
    ] {
        assert!(SpaceMode::Server.allows(feature).is_ok());
        let err = SpaceMode::EndToEnd.allows(feature).unwrap_err();
        let err: vellum_kernel::Error = err.into();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }
    let err: vellum_kernel::Error = E2eError::LastKey.into();
    assert_eq!(err.kind(), ErrorKind::Conflict);
}
