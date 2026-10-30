// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red-team regressions for identity (#259).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use std::time::Duration;

use url::Url;
use vellum_identity::oauth::{Authorize, AuthorizeRequest, TokenRequest};
use vellum_identity::{DESKTOP_CLIENT_ID, Identity, IdentityConfig, Store};
use vellum_kernel::Timestamp;

const PASSWORD: &str = "correct horse battery staple";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "http://127.0.0.1:49152/callback";
/// The admin operations' recent-sign-in window.
const FRESH: Duration = Duration::from_mins(15);

/// An admin-scope access token approved from a session that signed in
/// `signed_in_ago` ago.
async fn admin_token(identity: &Identity, signed_in_ago: Duration) -> String {
    let user = identity.users().first().unwrap().id;
    let request = AuthorizeRequest {
        response_type: Some("code".into()),
        client_id: Some(DESKTOP_CLIENT_ID.into()),
        redirect_uri: Some(REDIRECT.into()),
        scope: Some("read admin".into()),
        code_challenge: Some(CHALLENGE.into()),
        code_challenge_method: Some("S256".into()),
        ..AuthorizeRequest::default()
    };
    let Authorize::Consent(view) = identity.authorize(user, &request).await else {
        panic!("expected a consent screen");
    };
    let ago = i64::try_from(signed_in_ago.as_millis()).unwrap();
    let signed_in = Timestamp::from_unix_ms(Timestamp::now().unix_ms() - ago).unwrap();
    let back = identity
        .decide(user, Some(signed_in), &view.consent_id, true)
        .unwrap();
    let code = Url::parse(&back)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let tokens = identity
        .token(&TokenRequest {
            grant_type: Some("authorization_code".into()),
            code: Some(code),
            redirect_uri: Some(REDIRECT.into()),
            client_id: Some(DESKTOP_CLIENT_ID.into()),
            code_verifier: Some(VERIFIER.into()),
            ..TokenRequest::default()
        })
        .unwrap();
    tokens.access_token
}

/// RT-039: a session idle for hours (a stolen cookie) could approve an
/// `OAuth` client for the `admin` scope and get a token that counted as
/// signed in just now, skipping the re-authentication admin actions ask
/// for. The token now carries the approving session's sign-in time.
#[tokio::test]
async fn oauth_tokens_keep_the_approving_sessions_sign_in_time() {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let identity = Identity::new(IdentityConfig::new(url), Store::memory());
    let _ = identity
        .setup("priya@northstar.dev", "Priya", PASSWORD)
        .unwrap();

    let stale = admin_token(&identity, Duration::from_hours(3)).await;
    let principal = identity.bearer(&stale).unwrap().principal;
    assert!(
        principal.require_fresh(FRESH, Timestamp::now()).is_err(),
        "an old session's approval must not mint a fresh admin credential"
    );

    let recent = admin_token(&identity, Duration::from_mins(1)).await;
    let principal = identity.bearer(&recent).unwrap().principal;
    assert!(principal.require_fresh(FRESH, Timestamp::now()).is_ok());
}
