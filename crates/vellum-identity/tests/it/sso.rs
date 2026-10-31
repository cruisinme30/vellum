// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Single sign-on against an in-process provider that signs real tokens.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _,
    RSA_PKCS1_SHA256, RsaKeyPair,
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use url::Url;
use vellum_contracts::BoxFuture;
use vellum_identity::sso::{SsoFinish, SsoIdentity};
use vellum_identity::{
    Identity, IdentityConfig, Sso, SsoConfig, SsoHttp, Store,
};
use vellum_kernel::{Error, ErrorKind, Timestamp};

const ISSUER: &str = "https://id.northstar.dev";
const CLIENT: &str = "vellum";
const CALLBACK: &str = "https://vellum.northstar.dev/auth/sso/callback";

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The modulus and exponent of a PKCS#1 `RSAPublicKey` (DER): a
/// SEQUENCE of two INTEGERs.
fn rsa_components(der: &[u8]) -> (Vec<u8>, Vec<u8>) {
    fn read(der: &[u8]) -> (&[u8], &[u8]) {
        // Tag, then a short or long-form length.
        let (len, start) = match der[1] {
            l if l < 0x80 => (usize::from(l), 2),
            l => {
                let n = usize::from(l & 0x7f);
                let len = der[2..2 + n]
                    .iter()
                    .fold(0, |acc, b| acc * 256 + usize::from(*b));
                (len, 2 + n)
            }
        };
        (&der[start..start + len], &der[start + len..])
    }
    let (seq, _) = read(der);
    let (n, rest) = read(seq);
    let (e, _) = read(rest);
    let strip = |v: &[u8]| v.iter().skip_while(|b| **b == 0).copied().collect();
    (strip(n), strip(e))
}

/// Builds an ID token for a nonce.
type MakeToken<'a> = Box<dyn Fn(&str) -> String + 'a>;

/// A token request: the form and the basic credentials.
type Posted = (Vec<(String, String)>, Option<(String, String)>);

/// A provider: discovery, keys, and a token endpoint that returns the
/// next ID token the test prepared.
struct FakeIdp {
    ec: EcdsaKeyPair,
    rsa: RsaKeyPair,
    next_token: Mutex<Option<String>>,
    posted: Mutex<Vec<Posted>>,
}

impl FakeIdp {
    fn new() -> Self {
        Self {
            ec: EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING)
                .unwrap(),
            rsa: RsaKeyPair::generate(KeySize::Rsa2048).unwrap(),
            next_token: Mutex::new(None),
            posted: Mutex::new(Vec::new()),
        }
    }

    fn jwks(&self) -> Value {
        let point = self.ec.public_key().as_ref();
        let (n, e) = rsa_components(self.rsa.public_key().as_ref());
        json!({ "keys": [
            { "kty": "EC", "kid": "ec1", "use": "sig", "crv": "P-256",
              "x": b64(&point[1..33]), "y": b64(&point[33..65]) },
            { "kty": "RSA", "kid": "rsa1", "use": "sig", "alg": "RS256",
              "n": b64(&n), "e": b64(&e) },
        ]})
    }

    fn sign(&self, alg: &str, kid: &str, claims: &Value) -> String {
        let header = b64(json!({ "alg": alg, "kid": kid, "typ": "JWT" })
            .to_string()
            .as_bytes());
        let body = b64(claims.to_string().as_bytes());
        let message = format!("{header}.{body}");
        let rng = SystemRandom::new();
        let signature = if alg == "ES256" {
            self.ec
                .sign(&rng, message.as_bytes())
                .unwrap()
                .as_ref()
                .to_vec()
        } else {
            let mut sig = vec![0; self.rsa.public_modulus_len()];
            self.rsa
                .sign(&RSA_PKCS1_SHA256, &rng, message.as_bytes(), &mut sig)
                .unwrap();
            sig
        };
        format!("{message}.{}", b64(&signature))
    }

    fn will_issue(&self, token: String) {
        *self.next_token.lock().unwrap() = Some(token);
    }
}

impl SsoHttp for FakeIdp {
    fn get<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
        let body = match url {
            "https://id.northstar.dev/.well-known/openid-configuration" => {
                json!({
                    "issuer": ISSUER,
                    "authorization_endpoint": "https://id.northstar.dev/authori\
                        ze",
                    "token_endpoint": "https://id.northstar.dev/token",
                    "jwks_uri": "https://id.northstar.dev/jwks",
                })
            }
            "https://id.northstar.dev/jwks" => self.jwks(),
            _ => {
                return Box::pin(async {
                    Err(Error::new(ErrorKind::NotFound, "no such page"))
                });
            }
        };
        Box::pin(async move { Ok(body.to_string().into_bytes()) })
    }

    fn post_form<'a>(
        &'a self,
        _url: &'a str,
        form: Vec<(String, String)>,
        basic: Option<(String, String)>,
    ) -> BoxFuture<'a, Result<(u16, Vec<u8>), Error>> {
        self.posted.lock().unwrap().push((form, basic));
        let token = self.next_token.lock().unwrap().take();
        Box::pin(async move {
            Ok(token.map_or_else(
                || (400, br#"{"error":"invalid_grant"}"#.to_vec()),
                |t| {
                    let body = json!({ "id_token": t, "token_type": "Bearer" });
                    (200, body.to_string().into_bytes())
                },
            ))
        })
    }
}

fn config() -> SsoConfig {
    SsoConfig {
        issuer: Url::parse(ISSUER).unwrap(),
        client_id: CLIENT.into(),
        client_secret: Some("s3cret:with/odd chars".into()),
        display_name: "Northstar".into(),
        allowed_domains: vec!["northstar.dev".into()],
        auto_create: true,
    }
}

fn now() -> i64 {
    Timestamp::now().unix_ms() / 1000
}

fn claims(nonce: &str, email: &str) -> Value {
    json!({
        "iss": ISSUER, "aud": CLIENT, "sub": format!("sub-{email}"),
        "exp": now() + 300, "iat": now(), "nonce": nonce,
        "email": email, "email_verified": true, "name": "Ana Lima",
    })
}

fn param(url: &str, name: &str) -> String {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap()
}

/// Starts a sign-in, lets `token` shape the ID token, and finishes it.
async fn sign_in(
    sso: &Sso,
    idp: &FakeIdp,
    token: impl FnOnce(&str) -> String,
) -> Result<SsoFinish, Error> {
    let started = sso.start(CALLBACK, Some("/drive"), false).await?;
    idp.will_issue(token(&param(&started.url, "nonce")));
    sso.finish(CALLBACK, &started.state, "code-1", Some(&started.state))
        .await
}

fn setup() -> (Arc<FakeIdp>, Sso) {
    let idp = Arc::new(FakeIdp::new());
    let sso = Sso::new(config(), Arc::clone(&idp) as Arc<dyn SsoHttp>);
    (idp, sso)
}

#[tokio::test]
async fn a_signed_token_signs_in_and_returns_to_the_page() {
    let (idp, sso) = setup();
    for (alg, kid) in [("ES256", "ec1"), ("RS256", "rsa1")] {
        let done = sign_in(&sso, &idp, |n| {
            idp.sign(alg, kid, &claims(n, "Ana@Northstar.dev"))
        })
        .await
        .unwrap();
        assert_eq!(done.next, "/drive");
        assert_eq!(done.identity.email.as_deref(), Some("ana@northstar.dev"));
        assert!(done.identity.email_verified);
        assert_eq!(done.identity.subject, "sub-Ana@Northstar.dev");
    }
    // PKCE and client_secret_basic, with the secret form-encoded.
    let (form, basic) = idp.posted.lock().unwrap()[0].clone();
    assert!(form.iter().any(|(k, _)| k == "code_verifier"));
    assert_eq!(
        basic.as_ref().unwrap(),
        &("vellum".to_owned(), "s3cret%3Awith%2Fodd+chars".to_owned())
    );
}

#[tokio::test]
async fn the_sign_in_url_asks_for_a_code_with_pkce() {
    let (_, sso) = setup();
    let started = sso
        .start(CALLBACK, Some("//evil.example"), true)
        .await
        .unwrap();
    assert!(
        started
            .url
            .starts_with("https://id.northstar.dev/authorize?")
    );
    assert_eq!(param(&started.url, "response_type"), "code");
    assert_eq!(param(&started.url, "code_challenge_method"), "S256");
    assert_eq!(param(&started.url, "redirect_uri"), CALLBACK);
    assert_eq!(param(&started.url, "state"), started.state);
    assert_eq!(param(&started.url, "prompt"), "login");
}

#[tokio::test]
async fn forged_or_misdirected_tokens_are_refused() {
    let (idp, sso) = setup();
    let cases: Vec<(&str, MakeToken<'_>)> = vec![
        (
            "wrong nonce",
            Box::new(|_| {
                idp.sign("ES256", "ec1", &claims("other", "ana@northstar.dev"))
            }),
        ),
        (
            "other audience",
            Box::new(|n| {
                let mut c = claims(n, "ana@northstar.dev");
                c["aud"] = json!("someone-else");
                idp.sign("ES256", "ec1", &c)
            }),
        ),
        (
            "other issuer",
            Box::new(|n| {
                let mut c = claims(n, "ana@northstar.dev");
                c["iss"] = json!("https://evil.example");
                idp.sign("ES256", "ec1", &c)
            }),
        ),
        (
            "expired",
            Box::new(|n| {
                let mut c = claims(n, "ana@northstar.dev");
                c["exp"] = json!(now() - 3600);
                idp.sign("ES256", "ec1", &c)
            }),
        ),
        (
            "tampered",
            Box::new(|n| {
                let token =
                    idp.sign("ES256", "ec1", &claims(n, "ana@northstar.dev"));
                let parts: Vec<&str> = token.split('.').collect();
                let forged =
                    b64(claims(n, "boss@northstar.dev").to_string().as_bytes());
                format!("{}.{forged}.{}", parts[0], parts[2])
            }),
        ),
        (
            "alg none",
            Box::new(|n| {
                let header = b64(br#"{"alg":"none","kid":"ec1"}"#);
                let body =
                    b64(claims(n, "ana@northstar.dev").to_string().as_bytes());
                format!("{header}.{body}.")
            }),
        ),
        (
            "unknown key",
            Box::new(|n| {
                idp.sign("ES256", "nope", &claims(n, "ana@northstar.dev"))
            }),
        ),
        (
            "RSA key, EC header",
            Box::new(|n| {
                idp.sign("ES256", "rsa1", &claims(n, "ana@northstar.dev"))
            }),
        ),
    ];
    for (name, token) in cases {
        let err = sign_in(&sso, &idp, |n| token(n)).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthenticated, "{name}: {err}");
    }
}

#[tokio::test]
async fn a_callback_needs_this_browser_and_works_once() {
    let (idp, sso) = setup();
    let started = sso.start(CALLBACK, None, false).await.unwrap();
    let nonce = param(&started.url, "nonce");
    idp.will_issue(idp.sign(
        "ES256",
        "ec1",
        &claims(&nonce, "ana@northstar.dev"),
    ));
    let other_browser = sso
        .finish(CALLBACK, &started.state, "c", Some("other"))
        .await;
    assert_eq!(
        other_browser.unwrap_err().kind(),
        ErrorKind::Unauthenticated
    );
    let no_cookie = sso.finish(CALLBACK, &started.state, "c", None).await;
    assert_eq!(no_cookie.unwrap_err().kind(), ErrorKind::Unauthenticated);
    let ok = sso
        .finish(CALLBACK, &started.state, "c", Some(&started.state))
        .await;
    assert_eq!(ok.unwrap().next, "/");
    idp.will_issue(idp.sign(
        "ES256",
        "ec1",
        &claims(&nonce, "ana@northstar.dev"),
    ));
    let replay = sso
        .finish(CALLBACK, &started.state, "c", Some(&started.state))
        .await;
    assert_eq!(replay.unwrap_err().kind(), ErrorKind::Unauthenticated);
}

#[tokio::test]
async fn reauthentication_needs_a_fresh_provider_sign_in() {
    let (idp, sso) = setup();
    for (auth_time, ok) in [
        (None, false),
        (Some(now() - 3600), false),
        (Some(now()), true),
    ] {
        let started = sso.start(CALLBACK, None, true).await.unwrap();
        let mut c = claims(&param(&started.url, "nonce"), "ana@northstar.dev");
        if let Some(at) = auth_time {
            c["auth_time"] = json!(at);
        }
        idp.will_issue(idp.sign("ES256", "ec1", &c));
        let done = sso
            .finish(CALLBACK, &started.state, "c", Some(&started.state))
            .await;
        assert_eq!(done.is_ok(), ok, "{auth_time:?}");
    }
}

fn identity(sso_config: SsoConfig) -> Identity {
    let idp = Arc::new(FakeIdp::new());
    let config = IdentityConfig::new(
        Url::parse("https://vellum.northstar.dev").unwrap(),
    );
    Identity::new(config, Store::memory()).with_sso(Sso::new(sso_config, idp))
}

fn found(subject: &str, email: Option<&str>, verified: bool) -> SsoIdentity {
    SsoIdentity {
        issuer: ISSUER.into(),
        subject: subject.into(),
        email: email.map(str::to_owned),
        email_verified: verified,
        name: Some("Ana Lima".into()),
    }
}

#[test]
fn accounts_link_by_subject_or_verified_email() {
    let id = identity(config());
    let first = found("s1", Some("ana@northstar.dev"), true);
    assert_eq!(
        id.sso_sign_in(&first, None).unwrap_err().kind(),
        ErrorKind::Conflict,
        "no SSO before the first administrator exists"
    );
    let admin = id
        .setup("priya@northstar.dev", "Priya", "correct horse battery")
        .unwrap();

    // An existing account links by verified address, and keeps its rights.
    let linked = id
        .sso_sign_in(&found("s-priya", Some("PRIYA@northstar.dev"), true), None)
        .unwrap();
    assert_eq!(linked.session.user, admin.id);
    // Later the subject alone is enough, even if the address changed.
    let again = id
        .sso_sign_in(&found("s-priya", Some("p@other.dev"), true), None)
        .unwrap();
    assert_eq!(again.session.user, admin.id);

    // A new address in an allowed domain gets a member account.
    let created = id.sso_sign_in(&first, None).unwrap();
    let user = id.user(created.session.user).unwrap();
    assert!(!user.admin);
    assert_eq!(user.name, "Ana Lima");
    assert!(!user.has_password);

    // Unverified addresses, other domains and missing addresses are refused.
    for refused in [
        found("s2", Some("bo@northstar.dev"), false),
        found("s3", Some("bo@evil.dev"), true),
        found("s4", None, true),
    ] {
        let err = id.sso_sign_in(&refused, None).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied, "{refused:?}");
    }

    // A disabled account stays locked out.
    let _ = id.update_user(user.id, None, None, Some(true)).unwrap();
    assert_eq!(
        id.sso_sign_in(&first, None).unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
}

#[test]
fn without_auto_create_only_existing_accounts_sign_in() {
    let mut no_create = config();
    no_create.auto_create = false;
    let id = identity(no_create);
    let _ = id
        .setup("priya@northstar.dev", "Priya", "correct horse battery")
        .unwrap();
    let err = id
        .sso_sign_in(&found("s1", Some("ana@northstar.dev"), true), None)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    assert!(
        id.sso_sign_in(&found("s2", Some("priya@northstar.dev"), true), None)
            .is_ok()
    );
}

/// RT-043 (#547): a provider limited to some domains cannot take over an
/// account in another domain by asserting its address.
#[test]
fn a_provider_cannot_link_accounts_outside_its_domains() {
    let mut partner = config();
    partner.allowed_domains = vec!["partner.example".into()];
    let id = identity(partner);
    let admin = id
        .setup("priya@northstar.dev", "Priya", "correct horse battery")
        .unwrap();
    let err = id
        .sso_sign_in(&found("evil", Some("priya@northstar.dev"), true), None)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    assert!(id.user(admin.id).unwrap().admin);
    // Its own people still sign in and get accounts.
    let made = id
        .sso_sign_in(&found("p1", Some("lee@partner.example"), true), None)
        .unwrap();
    assert_ne!(made.session.user, admin.id);
    // Without a domain list, linking works as before.
    let mut open = config();
    open.allowed_domains = Vec::new();
    let id = identity(open);
    let admin = id
        .setup("priya@northstar.dev", "Priya", "correct horse battery")
        .unwrap();
    let linked = id
        .sso_sign_in(&found("s", Some("priya@northstar.dev"), true), None)
        .unwrap();
    assert_eq!(linked.session.user, admin.id);
}
