// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Second factors (issue #523, ADR 0069 D290): authenticator apps,
//! recovery codes, the workspace policy, lockout and re-authentication.
//! Passkey ceremonies are covered with a software authenticator in the
//! crate's own tests.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use url::Url;
use vellum_identity::mfa::{Proof, SignIn};
use vellum_identity::store::SecondFactorPolicy;
use vellum_identity::{Identity, IdentityConfig, Store, totp};
use vellum_kernel::{ErrorKind, Timestamp, UserId};

const PASSWORD: &str = "correct horse battery staple";
const PRIYA: &str = "priya@northstar.dev";

fn identity() -> Identity {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    Identity::new(IdentityConfig::new(url), Store::memory())
        .with_sealing_key(&[3; 32])
        .unwrap()
}

fn now_step() -> u64 {
    u64::try_from(Timestamp::now().unix_ms() / 1000).unwrap() / totp::PERIOD
}

fn code(secret: &str, step: u64) -> String {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(secret.as_bytes())
        .unwrap();
    totp::code(&bytes, step)
}

fn code_only(code: &str) -> Proof {
    Proof {
        code: Some(code.to_owned()),
        passkey: None,
    }
}

/// Sets up an authenticator app for `user` and returns its secret and
/// the recovery codes.
fn add_app(id: &Identity, user: UserId) -> (String, Vec<String>) {
    let setup = id.start_totp(user).unwrap();
    assert!(setup.uri.starts_with("otpauth://totp/Vellum"));
    assert!(setup.qr.len() >= 21);
    // Confirming takes the code for the step before now, so the sign-in
    // below can use the current step.
    let added = id
        .confirm_totp(user, &code(&setup.secret, now_step() - 1))
        .unwrap();
    assert!(added.factors.totp);
    (setup.secret, added.recovery_codes.unwrap())
}

fn ticket(outcome: SignIn) -> String {
    match outcome {
        SignIn::Verify(t) | SignIn::Enrol(t) => t.ticket,
        SignIn::Session(_) => panic!("expected a second step"),
    }
}

#[test]
fn a_password_alone_no_longer_signs_in_once_an_app_is_added() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    assert!(matches!(
        id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap(),
        SignIn::Session(_)
    ));
    let (secret, recovery) = add_app(&id, priya.id);
    assert_eq!(recovery.len(), 10);
    assert!(id.user(priya.id).unwrap().second_factor);

    // The plain sign-in refuses: it would skip the second step.
    let refused = id.sign_in(PRIYA, PASSWORD, None, None).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::PermissionDenied);

    let outcome = id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap();
    let SignIn::Verify(step) = outcome else {
        panic!("expected a second step");
    };
    assert_eq!(step.methods, ["totp", "recovery"]);
    let wrong = id
        .verify_sign_in(&step.ticket, &code_only("000000"), None)
        .unwrap_err();
    assert_eq!(wrong.kind(), ErrorKind::Unauthenticated);
    let used = code(&secret, now_step());
    let session = id
        .verify_sign_in(&step.ticket, &code_only(&used), None)
        .unwrap();
    assert!(id.session(&session.secret).is_some());

    // The ticket worked once, and the code cannot be used again.
    let again = id
        .verify_sign_in(&step.ticket, &code_only(&used), None)
        .unwrap_err();
    assert_eq!(again.kind(), ErrorKind::Unauthenticated);
    let replay = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    assert!(id.verify_sign_in(&replay, &code_only(&used), None).is_err());
}

#[test]
fn recovery_codes_work_once_each() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let (_, recovery) = add_app(&id, priya.id);
    let first = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    let typed = recovery[0].to_uppercase().replace('-', " ");
    assert!(id.verify_sign_in(&first, &code_only(&typed), None).is_ok());
    assert_eq!(id.factors(priya.id).unwrap().recovery_codes_left, 9);
    let second = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    assert!(
        id.verify_sign_in(&second, &code_only(&recovery[0]), None)
            .is_err()
    );
    let renewed = id.new_recovery_codes(priya.id).unwrap();
    assert_eq!(id.factors(priya.id).unwrap().recovery_codes_left, 10);
    let third = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    assert!(
        id.verify_sign_in(&third, &code_only(&recovery[1]), None)
            .is_err(),
        "the old set is gone"
    );
    let fourth = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    assert!(
        id.verify_sign_in(&fourth, &code_only(&renewed[0]), None)
            .is_ok()
    );
}

#[test]
fn guessing_codes_ends_the_ticket_and_locks_the_account() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let (secret, _) = add_app(&id, priya.id);
    let step = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    for guess in 0..5 {
        let error = id
            .verify_sign_in(&step, &code_only(&format!("{guess:06}")), None)
            .unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ErrorKind::Unauthenticated | ErrorKind::RateLimited
            ),
            "{error}"
        );
    }
    // Five wrong answers spent the ticket, so even the right code fails.
    let after = id
        .verify_sign_in(&step, &code_only(&code(&secret, now_step())), None)
        .unwrap_err();
    assert!(matches!(
        after.kind(),
        ErrorKind::Unauthenticated | ErrorKind::RateLimited
    ));
    // Wrong codes count with wrong passwords: one more guess on a new
    // ticket passes the free failures, and the address locks.
    let next = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    let _ = id.verify_sign_in(&next, &code_only("999999"), None);
    let locked = id
        .sign_in_password(PRIYA, PASSWORD, None, None)
        .unwrap_err();
    assert_eq!(locked.kind(), ErrorKind::RateLimited);
    let held = id
        .verify_sign_in(&next, &code_only(&code(&secret, now_step())), None)
        .unwrap_err();
    assert_eq!(held.kind(), ErrorKind::RateLimited);
}

#[test]
fn tickets_are_bound_to_their_purpose() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let _ = add_app(&id, priya.id);
    let step = ticket(id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap());
    // A sign-in ticket cannot enrol a new factor or be guessed into shape.
    assert!(id.ticket_totp_setup(&step).is_err());
    assert!(
        id.enrol_with_ticket(&step, Some("123456"), None, None)
            .is_err()
    );
    assert!(
        id.verify_sign_in("vmt_not-a-ticket", &code_only("123456"), None)
            .is_err()
    );
    let other = format!("vmt_{}", "A".repeat(43));
    assert!(
        id.verify_sign_in(&other, &code_only("123456"), None)
            .is_err()
    );
}

#[test]
fn the_policy_makes_people_enrol_before_their_session_opens() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    let _ = id
        .set_second_factor_policy(SecondFactorPolicy::Admins)
        .unwrap();
    assert!(matches!(
        id.sign_in_password("sam@northstar.dev", PASSWORD, None, None)
            .unwrap(),
        SignIn::Session(_)
    ));
    let outcome = id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap();
    let SignIn::Enrol(enrol) = outcome else {
        panic!("an admin must enrol");
    };
    let setup = id.ticket_totp_setup(&enrol.ticket).unwrap();
    assert!(
        id.enrol_with_ticket(&enrol.ticket, Some("000000"), None, None)
            .is_err()
    );
    let (session, codes) = id
        .enrol_with_ticket(
            &enrol.ticket,
            Some(&code(&setup.secret, now_step())),
            None,
            None,
        )
        .unwrap();
    assert!(id.session(&session.secret).is_some());
    assert_eq!(codes.len(), 10);
    // The policy keeps the last factor in place.
    let kept = id.remove_totp(priya.id).unwrap_err();
    assert_eq!(kept.kind(), ErrorKind::Conflict);

    let _ = id
        .set_second_factor_policy(SecondFactorPolicy::Everyone)
        .unwrap();
    assert!(matches!(
        id.sign_in_password("sam@northstar.dev", PASSWORD, None, None)
            .unwrap(),
        SignIn::Enrol(_)
    ));
    let _ = id
        .set_second_factor_policy(SecondFactorPolicy::Optional)
        .unwrap();
    let removed = id.remove_totp(priya.id).unwrap();
    assert!(!removed.totp);
    assert_eq!(removed.recovery_codes_left, 0);
    assert_eq!(sam.email, "sam@northstar.dev");
}

#[test]
fn re_authentication_asks_for_the_factor() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let before = id.sign_in(PRIYA, PASSWORD, None, None).unwrap();
    let (secret, recovery) = add_app(&id, priya.id);
    let none = Proof {
        code: None,
        passkey: None,
    };
    let missing = id
        .reauthenticate(&before.secret, PASSWORD, &none, None, None)
        .unwrap_err();
    assert_eq!(missing.message(), "second factor required");
    assert!(
        id.reauthenticate(&before.secret, PASSWORD, &code_only("999999"), None, None)
            .is_err()
    );
    assert!(
        id.reauthenticate(
            &before.secret,
            "wrong password!!",
            &code_only(&code(&secret, now_step())),
            None,
            None
        )
        .is_err()
    );
    id.reauthenticate(
        &before.secret,
        PASSWORD,
        &code_only(&code(&secret, now_step())),
        None,
        None,
    )
    .unwrap();
    id.reauthenticate(
        &before.secret,
        PASSWORD,
        &code_only(&recovery[3]),
        None,
        None,
    )
    .unwrap();
    assert_eq!(id.factors(priya.id).unwrap().recovery_codes_left, 9);
}

#[test]
fn an_admin_reset_clears_factors_and_sessions() {
    let id = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    let session = id
        .sign_in("sam@northstar.dev", PASSWORD, None, None)
        .unwrap();
    let _ = add_app(&id, sam.id);
    let view = id.reset_factors(sam.id).unwrap();
    assert!(!view.second_factor);
    assert!(id.session(&session.secret).is_none());
    assert!(matches!(
        id.sign_in_password("sam@northstar.dev", PASSWORD, None, None)
            .unwrap(),
        SignIn::Session(_)
    ));
    assert_eq!(id.sign_out_everywhere(sam.id).unwrap(), 1);
    assert_eq!(id.sign_out_everywhere(priya.id).unwrap(), 0);
}

#[test]
fn a_sealed_secret_needs_the_same_key() {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let first = Identity::new(
        IdentityConfig::new(url.clone()),
        Store::open(dir.path()).unwrap(),
    )
    .with_sealing_key(&[1; 32])
    .unwrap();
    let priya = first.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let (secret, _) = add_app(&first, priya.id);
    let text = std::fs::read_to_string(dir.path().join(".vellum/identity.json")).unwrap();
    assert!(!text.contains(&secret), "the secret is sealed at rest");
    drop(first);

    let other_key = Identity::new(IdentityConfig::new(url), Store::open(dir.path()).unwrap())
        .with_sealing_key(&[2; 32])
        .unwrap();
    let step = ticket(
        other_key
            .sign_in_password(PRIYA, PASSWORD, None, None)
            .unwrap(),
    );
    assert!(
        other_key
            .verify_sign_in(&step, &code_only(&code(&secret, now_step())), None)
            .is_err()
    );
}
