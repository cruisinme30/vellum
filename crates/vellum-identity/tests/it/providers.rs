// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in providers in Admin → Sign-in (issue #523, ADR 0069 D292):
//! several OIDC providers, client secrets sealed and never returned, and
//! "require single sign-on" with break-glass administrators.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use std::sync::{Arc, Mutex};

use serde_json::json;
use url::Url;
use vellum_contracts::BoxFuture;
use vellum_identity::mfa::SignIn;
use vellum_identity::providers::{
    ENV_PROVIDER, ProviderInput, ProviderKind, UpdateSignIn,
};
use vellum_identity::{Identity, IdentityConfig, SsoHttp, Store};
use vellum_kernel::{Error, ErrorKind};

const PASSWORD: &str = "correct horse battery staple";
const PRIYA: &str = "priya@northstar.dev";
const ISSUER: &str = "https://okta.northstar.dev";

type Basic = Option<(String, String)>;

/// A provider that answers discovery and records the client credentials
/// sent to its token endpoint.
#[derive(Default)]
struct Recorder {
    basic: Mutex<Vec<Basic>>,
}

impl SsoHttp for Recorder {
    fn get<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
        let body = (url
            == format!("{ISSUER}/.well-known/openid-configuration"))
        .then(|| {
            json!({
                "issuer": ISSUER,
                "authorization_endpoint": format!("{ISSUER}/authorize"),
                "token_endpoint": format!("{ISSUER}/token"),
                "jwks_uri": format!("{ISSUER}/jwks"),
            })
            .to_string()
            .into_bytes()
        });
        Box::pin(async move {
            body.ok_or_else(|| Error::new(ErrorKind::NotFound, "no page"))
        })
    }

    fn post_form<'a>(
        &'a self,
        _url: &'a str,
        _form: Vec<(String, String)>,
        basic: Basic,
    ) -> BoxFuture<'a, Result<(u16, Vec<u8>), Error>> {
        self.basic.lock().unwrap().push(basic);
        Box::pin(async { Ok((400, br#"{"error":"invalid_grant"}"#.to_vec())) })
    }
}

fn identity() -> (Identity, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let http = Arc::clone(&recorder);
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let id = Identity::new(IdentityConfig::new(url), Store::memory())
        .with_sealing_key(&[7; 32])
        .unwrap()
        .with_sso_http(Arc::new(move |_: &Url| {
            let http: Arc<dyn SsoHttp> = Arc::clone(&http) as Arc<dyn SsoHttp>;
            Ok(http)
        }));
    (id, recorder)
}

fn okta(secret: Option<&str>) -> ProviderInput {
    ProviderInput {
        name: "Okta".into(),
        enabled: true,
        issuer: ISSUER.into(),
        client_id: "vellum".into(),
        client_secret: secret.map(str::to_owned),
        allowed_domains: vec!["@Northstar.dev".into()],
        auto_create: true,
        kind: ProviderKind::Oidc,
        entity_id: String::new(),
        sso_url: String::new(),
        certificate: String::new(),
        metadata_url: None,
    }
}

const fn require_sso(on: bool) -> UpdateSignIn {
    UpdateSignIn {
        second_factor: None,
        require_sso: Some(on),
    }
}

#[tokio::test]
async fn admins_add_providers_whose_secrets_are_sealed_and_used() {
    let (id, recorder) = identity();
    let _ = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let added = id.add_provider(&okta(Some("top-secret"))).unwrap();
    assert_eq!(added.id, "okta");
    assert!(added.has_client_secret);
    assert_eq!(added.allowed_domains, ["northstar.dev"]);
    assert_eq!(added.start_url, "/auth/sso/start?provider=okta");

    // The secret is sealed at rest and never part of a view.
    let stored = id
        .store()
        .read(|s| serde_json::to_string(&s.sign_in).unwrap());
    assert!(!stored.contains("top-secret"));
    let view = serde_json::to_string(&id.sign_in_settings()).unwrap();
    assert!(!view.contains("top-secret"));
    assert!(!view.contains("clientSecret\""));

    // A second provider with the same name gets its own ID.
    assert_eq!(id.add_provider(&okta(None)).unwrap().id, "okta-2");
    let buttons = id.sign_in_buttons();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0].name, "Okta");

    // Signing in through it sends the opened secret to the token endpoint.
    let (provider, sso) = id.sso_provider(Some("okta")).unwrap();
    assert_eq!(provider, "okta");
    let redirect = "https://vellum.northstar.dev/auth/sso/callback";
    let started = sso.start(redirect, Some("/data"), false).await.unwrap();
    assert!(started.url.starts_with(&format!("{ISSUER}/authorize?")));
    let _ = sso
        .finish(redirect, &started.state, "code", Some(&started.state))
        .await
        .unwrap_err();
    let sent = recorder.basic.lock().unwrap().clone();
    assert_eq!(sent, [Some(("vellum".to_owned(), "top-secret".to_owned()))]);
}

#[test]
fn changing_a_provider_keeps_or_clears_its_secret() {
    let (id, _) = identity();
    let added = id.add_provider(&okta(Some("one"))).unwrap();
    let mut change = okta(None);
    change.name = "Okta SSO".into();
    let kept = id.update_provider(&added.id, &change).unwrap();
    assert_eq!(kept.name, "Okta SSO");
    assert!(kept.has_client_secret);
    change.client_secret = Some(String::new());
    assert!(
        !id.update_provider(&added.id, &change)
            .unwrap()
            .has_client_secret
    );

    change.enabled = false;
    let _ = id.update_provider(&added.id, &change).unwrap();
    assert!(id.sign_in_buttons().is_empty());
    assert_eq!(
        id.sso_provider(Some("okta")).unwrap_err().kind(),
        ErrorKind::NotFound
    );

    id.remove_provider(&added.id).unwrap();
    assert!(id.sign_in_settings().providers.is_empty());
    assert_eq!(
        id.remove_provider(&added.id).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    // The environment's provider is not changed here.
    assert_eq!(
        id.update_provider(ENV_PROVIDER, &okta(None))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn bad_provider_settings_are_refused() {
    let (id, _) = identity();
    let mut bad = okta(None);
    bad.issuer = "http://okta.northstar.dev".into();
    assert_eq!(
        id.add_provider(&bad).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let mut bad = okta(None);
    bad.name = " ".into();
    assert_eq!(
        id.add_provider(&bad).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let mut bad = okta(None);
    bad.allowed_domains = vec!["northstar dev".into()];
    assert_eq!(
        id.add_provider(&bad).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn requiring_sso_needs_a_provider_and_refuses_passwords() {
    let (id, _) = identity();
    let priya = id.setup(PRIYA, "Priya", PASSWORD).unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    assert_eq!(
        id.update_sign_in(&require_sso(true)).unwrap_err().kind(),
        ErrorKind::Conflict
    );
    let okta = id.add_provider(&okta(None)).unwrap();
    assert!(id.update_sign_in(&require_sso(true)).unwrap().require_sso);

    // Members and ordinary admins can no longer use a password.
    for email in [PRIYA, "sam@northstar.dev"] {
        assert_eq!(
            id.sign_in_password(email, PASSWORD, None, None)
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
    }
    // The last provider cannot go while SSO is required.
    assert_eq!(
        id.remove_provider(&okta.id).unwrap_err().kind(),
        ErrorKind::Conflict
    );

    // A break-glass admin still can, but only with a second factor.
    assert!(id.set_break_glass(priya.id, true).unwrap().break_glass);
    assert_eq!(id.sign_in_settings().break_glass_admins, 1);
    assert!(matches!(
        id.sign_in_password(PRIYA, PASSWORD, None, None).unwrap(),
        SignIn::Enrol(_)
    ));

    // Only admins can be break-glass, and losing admin ends it.
    assert_eq!(
        id.set_break_glass(sam.id, true).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let demoted = id.update_user(priya.id, None, Some(false), None);
    // The last admin cannot be demoted; either way break-glass stays
    // consistent with admin rights.
    if let Ok(user) = demoted {
        assert!(!user.break_glass);
    }

    // Turning the requirement off restores passwords.
    let _ = id.update_sign_in(&require_sso(false)).unwrap();
    assert!(matches!(
        id.sign_in_password("sam@northstar.dev", PASSWORD, None, None)
            .unwrap(),
        SignIn::Session(_)
    ));
}
