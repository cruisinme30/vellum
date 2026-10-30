// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Personal API tokens (issue #523, ADR 0069 D291): the admin expiry cap,
//! the admin list and revoke, and the scope ceiling.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use url::Url;
use vellum_contracts::Scope;
use vellum_identity::{Identity, IdentityConfig, Store};
use vellum_kernel::ErrorKind;

const PASSWORD: &str = "correct horse battery staple";

fn identity() -> Identity {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    Identity::new(IdentityConfig::new(url), Store::memory())
}

#[test]
fn the_admin_cap_bounds_new_tokens_only() {
    let id = identity();
    let priya = id.setup("priya@northstar.dev", "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    let (_, forever) = id
        .create_api_token(sam.id, "CI", &[Scope::Read], None)
        .unwrap();

    assert_eq!(id.set_max_token_days(Some(90)).unwrap(), Some(90));
    assert_eq!(id.max_token_days(), Some(90));
    let never = id
        .create_api_token(sam.id, "Laptop", &[Scope::Read], None)
        .unwrap_err();
    assert_eq!(never.kind(), ErrorKind::InvalidInput);
    let long = id
        .create_api_token(sam.id, "Laptop", &[Scope::Read], Some(91))
        .unwrap_err();
    assert_eq!(long.kind(), ErrorKind::InvalidInput);
    let (secret, ok) = id
        .create_api_token(sam.id, "Laptop", &[Scope::Read], Some(90))
        .unwrap();
    assert!(ok.expires_at.is_some());
    assert!(id.verify_token(&secret).is_ok());

    // The token made before the cap keeps working until an admin revokes it.
    let listed = id.all_api_tokens();
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|(_, owner)| owner.as_ref().unwrap().id == sam.id)
    );
    let _ = id.admin_revoke_api_token(&forever.id).unwrap();
    assert_eq!(id.api_tokens(sam.id).len(), 1);
    assert_eq!(
        id.admin_revoke_api_token(&forever.id).unwrap_err().kind(),
        ErrorKind::NotFound
    );

    assert_eq!(id.set_max_token_days(None).unwrap(), None);
    assert!(
        id.create_api_token(sam.id, "CI", &[Scope::Read], None)
            .is_ok()
    );
    assert_eq!(
        id.set_max_token_days(Some(0)).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        id.set_max_token_days(Some(3651)).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(priya.email, "priya@northstar.dev");
}

#[test]
fn people_grant_only_their_own_scopes() {
    let id = identity();
    let priya = id.setup("priya@northstar.dev", "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    assert!(id.token_scopes(priya.id).unwrap().contains(&Scope::Admin));
    let member = id.token_scopes(sam.id).unwrap();
    assert!(!member.contains(&Scope::Admin));
    assert!(member.contains(&Scope::Read) && member.contains(&Scope::Work));
    let denied = id
        .create_api_token(sam.id, "Root", &[Scope::Admin], Some(1))
        .unwrap_err();
    assert_eq!(denied.kind(), ErrorKind::PermissionDenied);
    let zero = id
        .create_api_token(sam.id, "Zero", &[Scope::Read], Some(0))
        .unwrap_err();
    assert_eq!(zero.kind(), ErrorKind::InvalidInput);
}

#[test]
fn blocking_a_person_ends_their_tokens() {
    let id = identity();
    let _ = id.setup("priya@northstar.dev", "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    let (secret, _) = id
        .create_api_token(sam.id, "CI", &[Scope::Read], Some(30))
        .unwrap();
    let _ = id.update_user(sam.id, None, None, Some(true)).unwrap();
    assert!(id.verify_token(&secret).is_err());
    assert!(id.all_api_tokens().is_empty());
}
