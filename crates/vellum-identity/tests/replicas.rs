// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Two replicas over one database (#434, ADR 0037 D160): a sign-in on one
//! works on the other, a sign-out or a disabled account takes effect on
//! both, lockout counts across them, and an `OAuth` code issued by one is
//! redeemed at the other, once.
//!
//! Each case runs on SQLite (two pools on one file, no Docker needed) and
//! on Postgres in a throwaway container.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use std::net::IpAddr;

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use url::Url;
use vellum_db::IndexConfig;
use vellum_identity::oauth::{Authorize, AuthorizeRequest, TokenRequest};
use vellum_identity::{DESKTOP_CLIENT_ID, Identity, IdentityConfig, Store};
use vellum_kernel::ErrorKind;

const PASSWORD: &str = "correct horse battery staple";
const EMAIL: &str = "priya@northstar.dev";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "http://127.0.0.1:49152/callback";

fn replica(config: &IndexConfig) -> Identity {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    Identity::new(
        IdentityConfig::new(url),
        Store::shared(config.clone()).unwrap(),
    )
}

#[allow(clippy::unnecessary_wraps, reason = "sign_in takes an optional IP")]
fn ip(last: u8) -> Option<IpAddr> {
    Some(IpAddr::from([10, 0, 0, last]))
}

fn sessions_and_lockout_are_shared(config: &IndexConfig) {
    let a = replica(config);
    let b = replica(config);
    assert!(a.needs_setup() && b.needs_setup());

    // An account made on A exists on B, and its address stays unique.
    let _ = a.setup(EMAIL, "Priya", PASSWORD).unwrap();
    assert!(!b.needs_setup());
    let taken = b.create_user(EMAIL, "Other", Some(PASSWORD), false);
    assert_eq!(taken.unwrap_err().kind(), ErrorKind::Conflict);

    // Signed in on A, signed in on B; signed out on B, signed out on A.
    let session = a.sign_in(EMAIL, PASSWORD, ip(1), Some("test")).unwrap();
    assert!(b.session(&session.secret).is_some());
    b.sign_out(&session.secret).unwrap();
    assert!(a.session(&session.secret).is_none());

    // Lockout counts failures from both: 3 + 3 is over the 5 free ones.
    for (replica, n) in [(&a, 3), (&b, 3)] {
        for _ in 0..n {
            let err = replica.sign_in(EMAIL, "wrong password!", ip(2), None);
            assert_eq!(err.unwrap_err().kind(), ErrorKind::Unauthenticated);
        }
    }
    let locked = a.sign_in(EMAIL, PASSWORD, ip(3), None);
    assert_eq!(locked.unwrap_err().kind(), ErrorKind::RateLimited);

    // A reopened replica (a restart) starts from the database.
    drop(b);
    let c = replica(config);
    assert_eq!(c.users().len(), 1);
}

async fn oauth_codes_cross_replicas_once(config: &IndexConfig) {
    let a = replica(config);
    let b = replica(config);
    let user = a.setup(EMAIL, "Priya", PASSWORD).unwrap().id;

    let request = AuthorizeRequest {
        response_type: Some("code".into()),
        client_id: Some(DESKTOP_CLIENT_ID.into()),
        redirect_uri: Some(REDIRECT.into()),
        scope: Some("read".into()),
        code_challenge: Some(CHALLENGE.into()),
        code_challenge_method: Some("S256".into()),
        ..AuthorizeRequest::default()
    };
    // The consent screen is shown by A and answered at B.
    let Authorize::Consent(view) = a.authorize(user, &request).await else {
        panic!("expected a consent screen");
    };
    let back =
        Url::parse(&b.decide(user, None, &view.consent_id, true).unwrap())
            .unwrap();
    let code = back
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap();

    // The code is redeemed at A, and only once anywhere.
    let exchange = TokenRequest {
        grant_type: Some("authorization_code".into()),
        code: Some(code),
        redirect_uri: Some(REDIRECT.into()),
        client_id: Some(DESKTOP_CLIENT_ID.into()),
        code_verifier: Some(VERIFIER.into()),
        ..TokenRequest::default()
    };
    let tokens = a.token(&exchange).unwrap();
    assert_eq!(b.token(&exchange).unwrap_err().error, "invalid_grant");

    // The access token minted on A is accepted by B.
    assert!(b.bearer(&tokens.access_token).is_ok());
}

fn sqlite(dir: &tempfile::TempDir) -> IndexConfig {
    IndexConfig::sqlite(dir.path().join("index.sqlite"), None)
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_replicas_share_sessions_and_lockout() {
    let dir = tempfile::tempdir().unwrap();
    sessions_and_lockout_are_shared(&sqlite(&dir));
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_replicas_share_oauth_codes() {
    let dir = tempfile::tempdir().unwrap();
    oauth_codes_cross_replicas_once(&sqlite(&dir)).await;
}

#[test]
fn the_store_works_outside_any_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let a = replica(&sqlite(&dir));
    let _ = a.setup(EMAIL, "Priya", PASSWORD).unwrap();
    let session = a.sign_in(EMAIL, PASSWORD, None, None).unwrap();
    assert!(replica(&sqlite(&dir)).session(&session.secret).is_some());
}

#[test]
fn a_single_binary_file_is_imported_once() {
    let data = tempfile::tempdir().unwrap();
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let file = Identity::new(
        IdentityConfig::new(url.clone()),
        Store::open(data.path()).unwrap(),
    );
    let _ = file.setup(EMAIL, "Priya", PASSWORD).unwrap();
    let session = file.sign_in(EMAIL, PASSWORD, None, None).unwrap();
    drop(file);

    let db = tempfile::tempdir().unwrap();
    let store = Store::shared(sqlite(&db)).unwrap();
    assert!(store.import_file(data.path()).unwrap());
    assert!(!store.import_file(data.path()).unwrap());
    assert!(data.path().join(".vellum/identity.json.imported").exists());
    let shared = Identity::new(IdentityConfig::new(url), store);
    assert!(shared.session(&session.secret).is_some());
}

async fn postgres() -> (ContainerAsync<Postgres>, IndexConfig) {
    let container = Postgres::default()
        .with_tag("17-alpine")
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, IndexConfig::postgres(url))
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_replicas_share_sessions_and_lockout() {
    let (_container, config) = postgres().await;
    sessions_and_lockout_are_shared(&config);
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_replicas_share_oauth_codes() {
    let (_container, config) = postgres().await;
    oauth_codes_cross_replicas_once(&config).await;
}

// The name starts with "postgres" so `cargo xtask fast` leaves it out
// where Docker is missing, like the other container tests.
#[tokio::test(flavor = "multi_thread")]
async fn postgres_concurrent_writers_on_two_replicas_lose_nothing() {
    let (_container, config) = postgres().await;
    let a = std::sync::Arc::new(replica(&config));
    let b = std::sync::Arc::new(replica(&config));
    let _ = a.setup(EMAIL, "Priya", PASSWORD).unwrap();
    let mut threads = Vec::new();
    for (n, identity) in [a.clone(), b.clone(), a.clone(), b.clone()]
        .into_iter()
        .enumerate()
    {
        threads.push(std::thread::spawn(move || {
            for i in 0..5 {
                let email = format!("user-{n}-{i}@northstar.dev");
                let _ = identity
                    .create_user(&email, "Someone", None, false)
                    .unwrap();
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(a.users().len(), 21);
    assert_eq!(b.users().len(), 21);
}
