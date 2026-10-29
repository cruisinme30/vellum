// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in, tokens, `OAuth` and break-glass through the real gateway.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tower::ServiceExt as _;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::services::{Services, VERSION};

pub const ORIGIN: &str = "http://localhost";

pub struct Reply {
    pub status: StatusCode,
    pub headers: http::HeaderMap,
    pub text: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }
}

pub fn app(data_dir: Option<&std::path::Path>) -> Router {
    let services = Services::open(ORIGIN, data_dir, None).unwrap();
    let mut config = GatewayConfig::new(ORIGIN, vec![3; 32]);
    config.rate_per_second = 10_000;
    config.rate_burst = 10_000;
    services
        .mount(GatewayBuilder::new(config, VERSION))
        .build()
        .unwrap()
        .router()
}

pub async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

pub fn request(method: &str, path: &str) -> http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
}

pub fn json_request(method: &str, path: &str, body: &Value) -> http::request::Builder {
    request(method, path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .header("x-body", body.to_string())
}

/// Builds a JSON request; the body travels in a header above only to keep
/// the builder chain readable, and is moved into the body here.
pub fn finish(builder: http::request::Builder) -> Request<Body> {
    let mut request = builder.body(Body::empty()).unwrap();
    if let Some(body) = request.headers_mut().remove("x-body") {
        *request.body_mut() = Body::from(body.as_bytes().to_vec());
    }
    request
}

pub fn cookie_of(reply: &Reply) -> String {
    let set = reply.headers[header::SET_COOKIE].to_str().unwrap();
    set.split(';').next().unwrap().to_owned()
}

pub async fn csrf(app: &Router, cookie: &str) -> String {
    let reply = send(
        app,
        finish(request("GET", "/api/csrf").header(header::COOKIE, cookie)),
    )
    .await;
    reply.json()["token"].as_str().unwrap().to_owned()
}

pub async fn signed_in_admin(app: &Router) -> String {
    let setup = send(
        app,
        finish(json_request(
            "POST",
            "/api/v1/setup",
            &json!({"email": "Priya@Northstar.dev", "name": "Priya", "password": "correct horse battery"}),
        )),
    )
    .await;
    assert_eq!(setup.status, StatusCode::OK, "{}", setup.text);
    let reply = send(
        app,
        finish(json_request(
            "POST",
            "/auth/sign-in",
            &json!({"email": "priya@northstar.dev", "password": "correct horse battery"}),
        )),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let set = reply.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Lax"),
        "{set}"
    );
    cookie_of(&reply)
}

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "one end-to-end story")]
async fn set_up_sign_in_and_manage_tokens() {
    let app = app(None);
    let status = send(&app, finish(request("GET", "/api/v1/auth/status"))).await;
    assert_eq!(status.json()["needsSetup"], true, "{}", status.text);
    assert_eq!(status.json()["credential"], "anonymous");

    let cookie = signed_in_admin(&app).await;
    let again = send(
        &app,
        finish(json_request(
            "POST",
            "/api/v1/setup",
            &json!({"email": "eve@northstar.dev", "name": "Eve", "password": "correct horse battery"}),
        )),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);

    let wrong = send(
        &app,
        finish(json_request(
            "POST",
            "/auth/sign-in",
            &json!({"email": "priya@northstar.dev", "password": "wrong horse battery"}),
        )),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let me = send(
        &app,
        finish(request("GET", "/api/v1/auth/status").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(me.json()["user"]["email"], "priya@northstar.dev");
    assert_eq!(me.json()["credential"], "session");

    // Writes from a cookie session need the CSRF token.
    let body = json!({"name": "CI", "scopes": ["read", "work"]});
    let refused = send(
        &app,
        finish(json_request("POST", "/api/v1/me/tokens", &body).header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let token_csrf = csrf(&app, &cookie).await;
    let created = send(
        &app,
        finish(
            json_request("POST", "/api/v1/me/tokens", &body)
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token_csrf),
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let token = created.json()["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("vlm_"));

    // The token works as a bearer, without CSRF, within its scopes.
    let bearer = format!("Bearer {token}");
    let status = send(
        &app,
        finish(request("GET", "/api/v1/auth/status").header(header::AUTHORIZATION, &bearer)),
    )
    .await;
    assert_eq!(status.json()["credential"], "api_token");
    let admin = send(
        &app,
        finish(request("GET", "/api/v1/admin/users").header(header::AUTHORIZATION, &bearer)),
    )
    .await;
    assert_eq!(admin.status, StatusCode::FORBIDDEN, "{}", admin.text);
    let mint = send(
        &app,
        finish(
            json_request("POST", "/api/v1/me/tokens", &body).header(header::AUTHORIZATION, &bearer),
        ),
    )
    .await;
    assert_eq!(
        mint.status,
        StatusCode::FORBIDDEN,
        "tokens cannot mint tokens"
    );
    let bogus = send(
        &app,
        finish(
            request("GET", "/api/v1/auth/status").header(header::AUTHORIZATION, "Bearer vlm_nope"),
        ),
    )
    .await;
    assert_eq!(bogus.status, StatusCode::UNAUTHORIZED);

    // The administrator, freshly signed in, can list users.
    let users = send(
        &app,
        finish(request("GET", "/api/v1/admin/users").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(users.status, StatusCode::OK, "{}", users.text);
    assert_eq!(users.json().as_array().map(Vec::len), Some(1));

    // Revoking the token ends it.
    let id = created.json()["details"]["id"].as_str().unwrap().to_owned();
    let revoked = send(
        &app,
        finish(
            request("DELETE", &format!("/api/v1/me/tokens/{id}"))
                .header(header::COOKIE, &cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &token_csrf),
        ),
    )
    .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text);
    let after = send(
        &app,
        finish(request("GET", "/api/v1/auth/status").header(header::AUTHORIZATION, &bearer)),
    )
    .await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);

    // Signing out ends the session; the stale cookie is simply anonymous.
    let out = send(
        &app,
        finish(
            json_request("POST", "/auth/sign-out", &json!({}))
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token_csrf),
        ),
    )
    .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT, "{}", out.text);
    let stale = send(
        &app,
        finish(request("GET", "/api/v1/auth/status").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(stale.json()["credential"], "anonymous");
}

#[tokio::test]
async fn repeated_failures_lock_the_address_out() {
    let app = app(None);
    let _cookie = signed_in_admin(&app).await;
    // The lock starts at one second after five failures and doubles with
    // each further failure. A slow machine can spend over a second on one
    // password check, so keep trying until the lock outlasts an attempt
    // rather than assuming the seventh try lands inside it.
    let mut statuses = Vec::new();
    for _ in 0..12 {
        let status = send(
            &app,
            finish(json_request(
                "POST",
                "/auth/sign-in",
                &json!({"email": "priya@northstar.dev", "password": "not the password"}),
            )),
        )
        .await
        .status;
        statuses.push(status);
        if status == StatusCode::TOO_MANY_REQUESTS {
            break;
        }
    }
    assert!(
        statuses[..5].iter().all(|s| *s == StatusCode::UNAUTHORIZED),
        "the first five failures are plain refusals: {statuses:?}"
    );
    assert_eq!(
        statuses.last(),
        Some(&StatusCode::TOO_MANY_REQUESTS),
        "{statuses:?}"
    );
}

fn pkce() -> (String, String) {
    let verifier = "M25iVXpKU3puUjFaYWg3T1NDTDQtcW1ROUY5YXlwalNoc0hhakxifmZHag".to_owned();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn query(url: &str) -> std::collections::HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

async fn token_request(app: &Router, form: &[(&str, &str)]) -> Reply {
    send(
        app,
        request("POST", "/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(serde_urlencoded::to_string(form).unwrap()))
            .unwrap(),
    )
    .await
}

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "one end-to-end story")]
async fn the_desktop_app_signs_in_with_oauth() {
    let app = app(None);
    let metadata = send(
        &app,
        finish(request("GET", "/.well-known/oauth-authorization-server")),
    )
    .await;
    assert_eq!(metadata.json()["issuer"], ORIGIN);
    assert_eq!(
        metadata.json()["authorization_response_iss_parameter_supported"],
        true
    );

    let (verifier, challenge) = pkce();
    let redirect = "http://127.0.0.1:53682/callback";
    let authorize = format!(
        "/oauth/authorize?{}",
        serde_urlencoded::to_string([
            ("response_type", "code"),
            ("client_id", "vellum-desktop"),
            ("redirect_uri", redirect),
            ("scope", "read write:direct work"),
            ("state", "s1"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", "http://localhost/mcp"),
        ])
        .unwrap()
    );
    let anonymous = send(&app, finish(request("GET", &authorize))).await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert!(
        anonymous.headers[header::LOCATION]
            .to_str()
            .unwrap()
            .starts_with("/sign-in?next=%2Foauth%2Fauthorize")
    );

    let cookie = signed_in_admin(&app).await;
    let page = send(
        &app,
        finish(request("GET", &authorize).header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert!(page.text.contains("Vellum for desktop"));
    let consent = page
        .text
        .split("data-consent=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    let token_csrf = csrf(&app, &cookie).await;
    let decided = send(
        &app,
        finish(
            json_request(
                "POST",
                "/auth/consent",
                &json!({"consentId": consent, "approve": true}),
            )
            .header(header::COOKIE, &cookie)
            .header("x-csrf-token", &token_csrf),
        ),
    )
    .await;
    assert_eq!(decided.status, StatusCode::OK, "{}", decided.text);
    let back = decided.json()["redirectTo"].as_str().unwrap().to_owned();
    assert!(back.starts_with(redirect));
    let params = query(&back);
    assert_eq!(params["state"], "s1");
    assert_eq!(params["iss"], ORIGIN);
    let code = params["code"].clone();

    let wrong = token_request(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect),
            ("client_id", "vellum-desktop"),
            ("code_verifier", "x".repeat(43).as_str()),
        ],
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong.json()["error"], "invalid_grant");
    // The failed attempt spent the code; start again.
    let page = send(
        &app,
        finish(request("GET", &authorize).header(header::COOKIE, &cookie)),
    )
    .await;
    let consent = page
        .text
        .split("data-consent=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    let decided = send(
        &app,
        finish(
            json_request(
                "POST",
                "/auth/consent",
                &json!({"consentId": consent, "approve": true}),
            )
            .header(header::COOKIE, &cookie)
            .header("x-csrf-token", &token_csrf),
        ),
    )
    .await;
    let code = query(decided.json()["redirectTo"].as_str().unwrap())["code"].clone();
    let tokens = token_request(
        &app,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect),
            ("client_id", "vellum-desktop"),
            ("code_verifier", &verifier),
        ],
    )
    .await;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text);
    assert_eq!(tokens.json()["scope"], "read write:direct work");
    let access = tokens.json()["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens.json()["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(tokens.headers[header::CACHE_CONTROL], "no-store");

    let status = send(
        &app,
        finish(
            request("GET", "/api/v1/auth/status")
                .header(header::AUTHORIZATION, format!("Bearer {access}")),
        ),
    )
    .await;
    assert_eq!(status.json()["user"]["name"], "Priya", "{}", status.text);

    let info = send(
        &app,
        finish(
            request("GET", "/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {access}")),
        ),
    )
    .await;
    assert_eq!(info.json()["email"], "priya@northstar.dev", "{}", info.text);
    assert!(metadata.json()["userinfo_endpoint"].is_string());

    let rotated = token_request(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", "vellum-desktop"),
        ],
    )
    .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    let new_access = rotated.json()["access_token"].as_str().unwrap().to_owned();

    // Replaying the spent refresh token revokes the whole grant.
    let replay = token_request(
        &app,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", "vellum-desktop"),
        ],
    )
    .await;
    assert_eq!(replay.json()["error"], "invalid_grant");
    let dead = send(
        &app,
        finish(
            request("GET", "/api/v1/auth/status")
                .header(header::AUTHORIZATION, format!("Bearer {new_access}")),
        ),
    )
    .await;
    assert_eq!(dead.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn break_glass_opens_a_short_admin_session_once() {
    let dir = std::env::temp_dir().join(format!("vellum-break-glass-{}", std::process::id()));
    let app = app(Some(&dir));
    let code = vellum_identity::Identity::issue_break_glass(&dir, Duration::from_mins(15)).unwrap();
    let redeem = || {
        finish(json_request(
            "POST",
            "/auth/break-glass",
            &json!({ "code": code }),
        ))
    };
    let first = send(&app, redeem()).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let cookie = cookie_of(&first);
    let status = send(
        &app,
        finish(request("GET", "/api/v1/auth/status").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(status.json()["credential"], "break_glass");
    assert!(
        status.json()["scopes"]
            .as_array()
            .unwrap()
            .contains(&json!("admin"))
    );
    let second = send(&app, redeem()).await;
    assert_eq!(second.status, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An anonymous caller gets 401 from the workspace services.
async fn anonymous_is_turned_away(app: &axum::Router) {
    let anonymous = send(app, finish(request("GET", "/api/v1/books"))).await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text
    );
}

/// Live editing is mounted. A test request carries no connection to
/// upgrade, so the socket handler answers 426 instead of a missing route.
async fn live_editing_is_mounted(app: &axum::Router) {
    let collab = format!(
        "/collab/{}/{}",
        vellum_kernel::SpaceId::generate(),
        vellum_kernel::DocId::generate()
    );
    let anonymous = send(
        app,
        finish(
            request("GET", &collab)
                .header(header::CONNECTION, "upgrade")
                .header(header::UPGRADE, "websocket")
                .header(header::SEC_WEBSOCKET_VERSION, "13")
                .header(header::SEC_WEBSOCKET_KEY, "AAAAAAAAAAAAAAAAAAAAAA=="),
        ),
    )
    .await;
    assert_eq!(
        anonymous.status,
        StatusCode::UPGRADE_REQUIRED,
        "{}",
        anonymous.text
    );
}

/// The person has a My Drive without asking for one, and a space's whole
/// tree comes in one request.
async fn drive_extras(app: &axum::Router, cookie: &str, space_id: &str, page_id: &str) {
    let mine = send(
        app,
        finish(request("GET", "/api/v1/drive/my-drive").header(header::COOKIE, cookie)),
    )
    .await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    assert_eq!(mine.json()["space"]["kind"], "user_drive", "{}", mine.text);
    let tree = send(
        app,
        finish(
            request("GET", &format!("/api/v1/drive/spaces/{space_id}/tree"))
                .header(header::COOKIE, cookie),
        ),
    )
    .await;
    assert_eq!(tree.status, StatusCode::OK, "{}", tree.text);
    assert_eq!(tree.json()["items"][0]["id"], page_id, "{}", tree.text);
}

/// `vellum serve` with a plaintext index: the workspace services answer a
/// signed-in user and turn away anonymous callers.
#[tokio::test(flavor = "multi_thread")]
async fn workspace_services_answer_signed_in_users() {
    let dir = std::env::temp_dir().join(format!(
        "vellum-workspace-test-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut config = vellum_kernel::Config::default();
    config.data_dir = dir.clone();
    let services = Services::open(ORIGIN, Some(&dir), None)
        .unwrap()
        .with_workspace(&config, |k| {
            (k == crate::workspace::PLAINTEXT_VAR).then(|| "1".to_owned())
        })
        .await
        .unwrap();
    let names: Vec<_> = services.all().iter().map(|s| s.name()).collect();
    assert!(
        ["drive", "collections", "books", "work", "search"]
            .iter()
            .all(|n| names.contains(n)),
        "{names:?}"
    );
    let mut gateway = GatewayConfig::new(ORIGIN, vec![3; 32]);
    gateway.rate_per_second = 10_000;
    gateway.rate_burst = 10_000;
    let app = services
        .mount(GatewayBuilder::new(gateway, VERSION))
        .build()
        .unwrap()
        .router();

    anonymous_is_turned_away(&app).await;
    live_editing_is_mounted(&app).await;

    let cookie = signed_in_admin(&app).await;
    for path in ["/api/v1/books", "/api/v1/collections", "/api/v1/work/teams"] {
        let reply = send(
            &app,
            finish(request("GET", path).header(header::COOKIE, &cookie)),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.text);
    }

    // A page saved through the API, then edited on disk by another
    // editor, gains a version by that editor once the watcher sees it.
    let token = csrf(&app, &cookie).await;
    let post = |path: &str, body: Value| {
        finish(
            json_request("POST", path, &body)
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token),
        )
    };
    let space = send(
        &app,
        post(
            "/api/v1/drive/spaces",
            json!({"kind": "shared_drive", "slug": "eng", "name": "Engineering"}),
        ),
    )
    .await;
    assert_eq!(space.status, StatusCode::OK, "{}", space.text);
    let space_id = space.json()["id"].as_str().unwrap().to_owned();
    let page = send(
        &app,
        post(
            &format!("/api/v1/drive/spaces/{space_id}/pages"),
            json!({"title": "Runbook", "body": "First draft.\n"}),
        ),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    let page_id = page.json()["id"].as_str().unwrap().to_owned();

    drive_extras(&app, &cookie, &space_id, &page_id).await;

    let file = dir.join(page.json()["path"].as_str().unwrap());
    let versions = format!("/api/v1/drive/items/{page_id}/versions");
    let mut seen = String::new();
    // An edit landing in the same burst as Vellum's own write is folded
    // into it, so edit again until the watcher has seen the page.
    'edits: for attempt in 0..10 {
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, format!("{text}Edit {attempt} in another editor.\n")).unwrap();
        for _ in 0..20 {
            let reply = send(
                &app,
                finish(request("GET", &versions).header(header::COOKIE, &cookie)),
            )
            .await;
            seen = reply.text;
            if seen.contains("external") {
                break 'edits;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    assert!(seen.contains("external"), "no external version: {seen}");

    // Search finds the edit made outside Vellum.
    let found = search_until(&app, &cookie, "another%20editor", &page_id).await;
    assert!(found.contains(&page_id), "page not found: {found}");
    services.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_pwa_registers_and_removes_a_browser_for_push() {
    let app = app(None);
    // Signed out, the key is not handed out.
    let anonymous = send(&app, finish(request("GET", "/api/v1/push/vapid-key"))).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let cookie = signed_in_admin(&app).await;
    let key = send(
        &app,
        finish(request("GET", "/api/v1/push/vapid-key").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(key.status, StatusCode::OK, "{}", key.text);
    let public_key = key.json()["publicKey"].as_str().unwrap().to_owned();
    assert_eq!(public_key.len(), 87, "base64url of 65 bytes: {public_key}");

    // What `PushSubscription.toJSON()` sends; any P-256 point will do.
    let token = csrf(&app, &cookie).await;
    let endpoint = "https://fcm.googleapis.com/fcm/send/abc123";
    let subscription = json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": {"p256dh": public_key, "auth": "BwcHBwcHBwcHBwcHBwcHBw"},
    });
    let call = |method: &str, path: &str, body: &Value| {
        finish(
            json_request(method, path, body)
                .header(header::COOKIE, &cookie)
                .header("x-csrf-token", &token),
        )
    };
    let added = send(
        &app,
        call("POST", "/api/v1/push/subscriptions", &subscription),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
    assert_eq!(added.json()["devices"], 1);
    // Registering the same browser again does not add a device.
    let again = send(
        &app,
        call("POST", "/api/v1/push/subscriptions", &subscription),
    )
    .await;
    assert_eq!(again.json()["devices"], 1);

    let plain = json!({"endpoint": "http://10.0.0.1/hook", "keys": subscription["keys"]});
    let refused = send(&app, call("POST", "/api/v1/push/subscriptions", &plain)).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    let query = serde_urlencoded::to_string([("endpoint", endpoint)]).unwrap();
    let removed = send(
        &app,
        call(
            "DELETE",
            &format!("/api/v1/push/subscriptions?{query}"),
            &json!({}),
        ),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.text);
    assert_eq!(removed.json()["devices"], 0);

    let prefs = send(
        &app,
        finish(request("GET", "/api/v1/notifications/preferences").header(header::COOKIE, &cookie)),
    )
    .await;
    assert_eq!(prefs.status, StatusCode::OK, "{}", prefs.text);
    let mut view = prefs.json();
    view["channels"]["commented"] = json!(["push"]);
    let saved = send(
        &app,
        call("PUT", "/api/v1/notifications/preferences", &view),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json()["channels"]["commented"], json!(["push"]));
}

#[tokio::test]
async fn a_mention_reaches_the_mentioned_persons_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = vellum_kernel::Config::default();
    config.data_dir = dir.path().to_path_buf();
    let services = Services::open(ORIGIN, Some(dir.path()), None)
        .unwrap()
        .with_workspace(&config, |k| {
            (k == crate::workspace::PLAINTEXT_VAR).then(|| "1".to_owned())
        })
        .await
        .unwrap();
    let mut gateway = GatewayConfig::new(ORIGIN, vec![3; 32]);
    gateway.rate_per_second = 10_000;
    gateway.rate_burst = 10_000;
    let app = services
        .mount(GatewayBuilder::new(gateway, VERSION))
        .build()
        .unwrap()
        .router();

    let priya = signed_in_admin(&app).await;
    let token = csrf(&app, &priya).await;
    let post = |path: &str, body: Value| {
        finish(
            json_request("POST", path, &body)
                .header(header::COOKIE, &priya)
                .header("x-csrf-token", &token),
        )
    };
    for (path, body) in [
        (
            "/api/v1/admin/users",
            json!({"email": "bo@northstar.dev", "name": "Bo", "password": "battery horse staple"}),
        ),
        (
            "/api/v1/work/teams",
            json!({"key": "ENG", "name": "Engineering"}),
        ),
        (
            "/api/v1/work/issues",
            json!({"team": "ENG", "title": "Fix login"}),
        ),
        (
            "/api/v1/work/comments",
            json!({"target": "ENG-1", "body": "@bo can you look at this?"}),
        ),
    ] {
        let reply = send(&app, post(path, body)).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.text);
    }

    let signed_in = send(
        &app,
        finish(json_request(
            "POST",
            "/auth/sign-in",
            &json!({"email": "bo@northstar.dev", "password": "battery horse staple"}),
        )),
    )
    .await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    let bo = cookie_of(&signed_in);

    // Notifications are delivered in the background, just after the write.
    let mut inbox = Value::Null;
    for _ in 0..50 {
        inbox = send(
            &app,
            finish(request("GET", "/api/v1/notifications").header(header::COOKIE, &bo)),
        )
        .await
        .json();
        if inbox["unread"] == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(inbox["unread"], 1, "{inbox}");
    let item = &inbox["items"][0];
    assert_eq!(item["kind"], "mentioned");
    assert_eq!(item["subject"], "ENG-1");
    assert_eq!(item["actor"], "priya");

    // The author is not told about their own comment.
    let own = send(
        &app,
        finish(request("GET", "/api/v1/notifications").header(header::COOKIE, &priya)),
    )
    .await
    .json();
    assert_eq!(own["unread"], 0, "{own}");

    let bo_token = csrf(&app, &bo).await;
    let read = send(
        &app,
        finish(
            json_request("POST", "/api/v1/notifications/read", &json!({"all": true}))
                .header(header::COOKIE, &bo)
                .header("x-csrf-token", &bo_token),
        ),
    )
    .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    assert_eq!(read.json()["unread"], 0);
}

/// Searches as the cookie's owner until a hit mentions `want`, for up to
/// ten seconds; returns the last reply.
async fn search_until(app: &Router, cookie: &str, q: &str, want: &str) -> String {
    let mut last = String::new();
    for _ in 0..100 {
        let reply = send(
            app,
            finish(request("GET", &format!("/api/v1/search?q={q}")).header(header::COOKIE, cookie)),
        )
        .await;
        last = reply.text;
        if last.contains(want) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    last
}
