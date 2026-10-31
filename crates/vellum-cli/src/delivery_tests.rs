// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The webhooks admin and the unsubscribe route through the real gateway.

use http::{StatusCode, header};
use serde_json::json;

use crate::identity_tests::{
    ORIGIN, Reply, app, csrf, finish, json_request, request, send,
    signed_in_admin,
};

async fn call(
    app: &axum::Router,
    cookie: &str,
    csrf: &str,
    method: &str,
    path: &str,
    body: &serde_json::Value,
) -> Reply {
    send(
        app,
        finish(
            json_request(method, path, body)
                .header(header::COOKIE, cookie)
                .header("x-csrf-token", csrf),
        ),
    )
    .await
}

#[tokio::test]
async fn admins_manage_webhooks_and_secrets_are_shown_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(Some(dir.path()));
    let cookie = signed_in_admin(&app).await;
    let token = csrf(&app, &cookie).await;

    let refused = call(
        &app,
        &cookie,
        &token,
        "POST",
        "/api/v1/admin/webhooks",
        &json!({"url": "https://127.0.0.1/hook", "event_types": ["issue"]}),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    let created = call(
        &app,
        &cookie,
        &token,
        "POST",
        "/api/v1/admin/webhooks",
        &json!({
            "url": "https://hooks.northstar.dev/vellum",
            "description": "CI",
            "event_types": ["issue", "comment"],
            "teams": ["eng"]
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    let secret = created.json()["secret"].as_str().unwrap().to_owned();
    let id = created.json()["hook"]["id"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("whsec_"));

    let listed = send(
        &app,
        finish(
            request("GET", "/api/v1/admin/webhooks")
                .header(header::COOKIE, &cookie),
        ),
    )
    .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert!(!listed.text.contains(&secret), "the secret is never listed");
    assert_eq!(listed.json()["hooks"][0]["teams"], json!(["ENG"]));
    assert_eq!(listed.json()["event_types"].as_array().unwrap().len(), 8);

    let updated = call(
        &app,
        &cookie,
        &token,
        "PUT",
        &format!(
            "/api/v1/admin/we\
        bhooks/{id}"
        ),
        &json!({
            "url": "https://hooks.northstar.dev/v2",
            "event_types": ["project"],
            "enabled": false
        }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json()["enabled"], json!(false));
    assert_eq!(updated.json()["teams"], json!([]));

    let rotated = call(
        &app,
        &cookie,
        &token,
        "POST",
        &format!("/api/v1/admin/webhooks/{id}/secret"),
        &json!({}),
    )
    .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    assert_ne!(rotated.json()["secret"].as_str().unwrap(), secret);
    assert_eq!(rotated.json()["hook"]["rotating"], json!(true));

    let log = send(
        &app,
        finish(
            request("GET", &format!("/api/v1/admin/webhooks/{id}/deliveries"))
                .header(header::COOKIE, &cookie),
        ),
    )
    .await;
    assert_eq!(log.status, StatusCode::OK, "{}", log.text);
    assert_eq!(log.json()["deliveries"], json!([]));

    // Kept across a restart, readable only by the server.
    let stored =
        std::fs::read_to_string(dir.path().join(".vellum/webhooks.json"))
            .unwrap();
    assert!(stored.contains(&id));

    let deleted = send(
        &app,
        finish(
            request("DELETE", &format!("/api/v1/admin/webhooks/{id}"))
                .header(header::COOKIE, &cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &token),
        ),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text);
    let gone = send(
        &app,
        finish(
            request("GET", &format!("/api/v1/admin/webhooks/{id}/deliveries"))
                .header(header::COOKIE, &cookie),
        ),
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    // Signed out, nothing.
    let anonymous =
        send(&app, finish(request("GET", "/api/v1/admin/webhooks"))).await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text
    );
}

#[tokio::test]
async fn forged_unsubscribe_links_change_nothing() {
    let app = app(None);
    let get = send(
        &app,
        finish(request(
            "GET",
            "/unsubscribe?u=00000000-0000-0000-0000-000000000000&s=ENG-1&t=AAAA",
        )),
    )
    .await;
    assert_eq!(get.status, StatusCode::BAD_REQUEST, "{}", get.text);
    assert!(get.text.contains("not valid"));
    let post = send(
        &app,
        finish(request("POST", "/unsubscribe?u=nope&s=ENG-1&t=AAAA")),
    )
    .await;
    assert_eq!(post.status, StatusCode::BAD_REQUEST, "{}", post.text);
}
