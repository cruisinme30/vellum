// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Single sign-on through the real gateway, against a provider listening
//! on this machine.

use std::sync::{Arc, Mutex};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _,
};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::{StatusCode, header};
use serde_json::{Value, json};
use url::Url;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::identity::oidc_config;
use crate::identity_tests::{
    ORIGIN, Reply, finish, request, send, signed_in_admin,
};
use crate::services::{Services, VERSION};

struct Provider {
    issuer: String,
    key: EcdsaKeyPair,
    nonce: Mutex<String>,
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn discovery(State(p): State<Arc<Provider>>) -> Json<Value> {
    Json(json!({
        "issuer": p.issuer,
        "authorization_endpoint": format!("{}/authorize", p.issuer),
        "token_endpoint": format!("{}/token", p.issuer),
        "jwks_uri": format!("{}/jwks", p.issuer),
    }))
}

async fn jwks(State(p): State<Arc<Provider>>) -> Json<Value> {
    let point = p.key.public_key().as_ref();
    Json(json!({ "keys": [{
        "kty": "EC", "kid": "k1", "crv": "P-256",
        "x": b64(&point[1..33]), "y": b64(&point[33..65]),
    }]}))
}

async fn token(
    State(p): State<Arc<Provider>>,
    headers: http::HeaderMap,
    body: String,
) -> Json<Value> {
    // The client authenticates with HTTP basic and proves PKCE.
    assert!(headers.contains_key(header::AUTHORIZATION));
    assert!(body.contains("code_verifier="), "{body}");
    let now = vellum_kernel::Timestamp::now().unix_ms() / 1000;
    let claims = json!({
        "iss": p.issuer, "aud": "vellum", "sub": "okta-ana",
        "exp": now + 300, "iat": now, "nonce": *p.nonce.lock().unwrap(),
        "email": "ana@northstar.dev", "email_verified": true, "name": "Ana Lima",
    });
    let header = b64(br#"{"alg":"ES256","kid":"k1"}"#);
    let message = format!("{header}.{}", b64(claims.to_string().as_bytes()));
    let signature = p
        .key
        .sign(&SystemRandom::new(), message.as_bytes())
        .unwrap();
    Json(
        json!({ "id_token": format!("{message}.{}", b64(signature.as_ref())) }),
    )
}

/// Starts the provider and returns it with its issuer URL.
async fn provider() -> Arc<Provider> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let provider = Arc::new(Provider {
        issuer,
        key: EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap(),
        nonce: Mutex::new(String::new()),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(Arc::clone(&provider));
    drop(tokio::spawn(
        async move { axum::serve(listener, app).await },
    ));
    provider
}

fn location(reply: &Reply) -> String {
    reply.headers[header::LOCATION].to_str().unwrap().to_owned()
}

fn set_cookies(reply: &Reply) -> Vec<String> {
    reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn single_sign_on_opens_a_session_and_returns_to_the_page() {
    let idp = provider().await;
    let sso = oidc_config(|k| match k {
        "VELLUM_OIDC_ISSUER" => Some(idp.issuer.clone()),
        "VELLUM_OIDC_CLIENT_ID" => Some("vellum".into()),
        "VELLUM_OIDC_CLIENT_SECRET" => Some("secret".into()),
        "VELLUM_OIDC_NAME" => Some("Northstar".into()),
        "VELLUM_OIDC_ALLOWED_DOMAINS" => Some("northstar.dev".into()),
        _ => None,
    })
    .unwrap();
    let services = Services::open(ORIGIN, None, sso).unwrap();
    let mut gateway = GatewayConfig::new(ORIGIN, vec![3; 32]);
    gateway.rate_per_second = 10_000;
    gateway.rate_burst = 10_000;
    let app = services
        .mount(GatewayBuilder::new(gateway, VERSION))
        .build()
        .unwrap()
        .router();
    let _admin = signed_in_admin(&app).await;

    // The sign-in page learns there is a button.
    let status =
        send(&app, finish(request("GET", "/api/v1/auth/status"))).await;
    assert_eq!(status.json()["sso"]["name"], "Northstar", "{}", status.text);

    let start =
        send(&app, finish(request("GET", "/auth/sso/start?next=/drive"))).await;
    assert_eq!(start.status, StatusCode::SEE_OTHER, "{}", start.text);
    let to_provider = Url::parse(&location(&start)).unwrap();
    assert!(
        to_provider
            .as_str()
            .starts_with(&format!("{}/authorize?", idp.issuer))
    );
    let param = |name: &str| {
        to_provider
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    *idp.nonce.lock().unwrap() = param("nonce");
    let state_cookie = set_cookies(&start).remove(0);
    assert!(state_cookie.starts_with("vellum_sso="), "{state_cookie}");
    let callback = format!(
        "/auth/sso/callback?code=c1&state={}",
        url::form_urlencoded::byte_serialize(param("state").as_bytes())
            .collect::<String>()
    );

    // Without the state cookie (another browser), the callback fails.
    let forged = send(&app, finish(request("GET", &callback))).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert!(forged.text.contains("another browser"), "{}", forged.text);

    // A fresh start, finished in the same browser, signs Ana in.
    let start =
        send(&app, finish(request("GET", "/auth/sso/start?next=/drive"))).await;
    let to_provider = Url::parse(&location(&start)).unwrap();
    let param = |name: &str| {
        to_provider
            .query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    *idp.nonce.lock().unwrap() = param("nonce");
    let state_cookie = set_cookies(&start).remove(0);
    let callback = format!(
        "/auth/sso/callback?code=c2&state={}",
        url::form_urlencoded::byte_serialize(param("state").as_bytes())
            .collect::<String>()
    );
    let done = send(
        &app,
        finish(request("GET", &callback).header(header::COOKIE, &state_cookie)),
    )
    .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.text);
    assert_eq!(location(&done), "/drive");
    let session = set_cookies(&done)
        .into_iter()
        .find(|c| {
            c.starts_with("vellum_session=")
                && c.len() > "vellum_session=".len()
        })
        .expect("a session cookie");
    let me = send(
        &app,
        finish(
            request("GET", "/api/v1/auth/status")
                .header(header::COOKIE, &session),
        ),
    )
    .await;
    assert_eq!(
        me.json()["user"]["email"],
        "ana@northstar.dev",
        "{}",
        me.text
    );
    assert_eq!(me.json()["user"]["admin"], false);
}
