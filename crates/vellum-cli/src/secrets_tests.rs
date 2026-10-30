// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secret scanning on save inside `vellum serve` (R-SE-6, D296, D301):
//! each write path under each policy (pages, models, text uploads and
//! collection rows), and the secret never leaving the server in errors,
//! audit records or logs.

use axum::Router;
use http::{StatusCode, header};
use serde_json::{Value, json};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::identity_tests::{ORIGIN, Reply, finish, request, send};
use crate::mcp_tests::{World, tool, world};
use crate::services::{Services, VERSION};

/// A GitHub token shaped string, built so no source file holds one.
fn token() -> String {
    format!("ghp_{}", "Zq9Xw8Vu7Ts6Rq5Po4Nm3Lk2Ji1Hg0FeDcBa")
}

/// A server over a fresh plaintext workspace, with the admin console.
async fn server(dir: &std::path::Path) -> (Services, Router) {
    let mut config = vellum_kernel::Config::default();
    config.data_dir = dir.to_path_buf();
    let plaintext = |k: &str| (k == crate::workspace::PLAINTEXT_VAR).then(|| "1".to_owned());
    let services = Services::open(ORIGIN, Some(dir), None)
        .unwrap()
        .with_workspace(&config, plaintext)
        .await
        .unwrap()
        .with_admin(&config, crate::observability::Observability::new(), |_| {
            None
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
    (services, app)
}

/// Sets the workspace's secret-scanning policy as the admin.
async fn set_policy(app: &Router, w: &World, policy: &str, allow: &[&str]) {
    let current = send(
        app,
        finish(request("GET", "/api/v1/admin/settings").header(header::COOKIE, &w.cookie)),
    )
    .await;
    assert_eq!(current.status, StatusCode::OK, "{}", current.text);
    let mut body = current.json();
    body["settings"]["security"]["secret_scanning"] = json!(policy);
    body["settings"]["security"]["secret_allow"] = json!(allow);
    let update = json!({"expected_version": body["version"], "settings": body["settings"]});
    let mut put = w.post("/api/v1/admin/settings", &update);
    *put.method_mut() = http::Method::PUT;
    let saved = send(app, put).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
}

async fn new_space(app: &Router, w: &World) -> String {
    let space = send(
        app,
        w.post(
            "/api/v1/drive/spaces",
            &json!({"kind": "shared_drive", "slug": "ops", "name": "Operations"}),
        ),
    )
    .await;
    assert_eq!(space.status, StatusCode::OK, "{}", space.text);
    space.json()["id"].as_str().unwrap().to_owned()
}

async fn create(app: &Router, w: &World, space: &str, title: &str, body: &str) -> Reply {
    send(
        app,
        w.post(
            &format!("/api/v1/drive/spaces/{space}/pages"),
            &json!({"title": title, "body": body}),
        ),
    )
    .await
}

async fn save(app: &Router, w: &World, body: &str) -> Reply {
    let id = &w.page_id;
    let current = send(
        app,
        finish(
            request("GET", &format!("/api/v1/drive/items/{id}/content"))
                .header(header::COOKIE, &w.cookie),
        ),
    )
    .await;
    assert_eq!(current.status, StatusCode::OK, "{}", current.text);
    let current = current.json();
    let mut put = w.post(
        &format!("/api/v1/drive/items/{id}/content"),
        &json!({
            "content": format!("---\ntitle: Runbook\n---\n\n{body}"),
            "base_content": current["content"],
            "base_etag": current["etag"],
        }),
    );
    *put.method_mut() = http::Method::PUT;
    send(app, put).await
}

async fn scan(app: &Router, w: &World, text: &str) -> Value {
    let reply = send(
        app,
        w.post("/api/v1/security/secrets/scan", &json!({ "text": text })),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    reply.json()
}

async fn audit_text(app: &Router, w: &World) -> String {
    let reply = send(
        app,
        finish(request("GET", "/api/v1/admin/audit").header(header::COOKIE, &w.cookie)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    reply.text
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_warn_by_default_block_when_asked_and_never_leak() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let space = new_space(&app, &w).await;
    let secret = token();
    let body = format!("The deploy bot pushes with {secret}.\n");

    // Warn (the default): the save goes through, the scan names the kind
    // and line, and the audit log records it without the value.
    let warned = create(&app, &w, &space, "Bot", &body).await;
    assert_eq!(warned.status, StatusCode::OK, "{}", warned.text);
    let found = scan(&app, &w, &body).await;
    assert_eq!(found["policy"], "warn");
    assert_eq!(found["findings"][0]["rule"], "github-token");
    assert_eq!(found["findings"][0]["line"], 1);
    let audit = audit_text(&app, &w).await;
    assert!(audit.contains("security.secret_found"), "{audit}");
    assert!(!audit.contains(&secret));
    assert_eq!(save(&app, &w, &body).await.status, StatusCode::OK);

    // Block: creating, saving and model drafts are all refused with a
    // typed 422 that names the kind and line, never the secret.
    set_policy(&app, &w, "block", &[]).await;
    for refused in [
        create(&app, &w, &space, "Bot 2", &body).await,
        save(&app, &w, &format!("Intro.\n\n{body}")).await,
    ] {
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            refused.text
        );
        let message = refused.json()["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(message.starts_with("secrets found"), "{message}");
        assert!(message.contains("github-token on line"), "{message}");
        assert!(!refused.text.contains(&secret), "{}", refused.text);
    }
    let drafted = tool(
        &app,
        &w.bearer,
        "create_draft",
        json!({"title": "Bot 3", "markdown": body, "parent": "space:ops"}),
    )
    .await;
    assert!(
        drafted.json()["result"]["isError"] == json!(true) || drafted.status.is_client_error(),
        "{}",
        drafted.text
    );
    assert!(!drafted.text.contains(&secret), "{}", drafted.text);
    assert_eq!(scan(&app, &w, &body).await["policy"], "block");
    // Pages without secrets still save.
    assert_eq!(
        save(&app, &w, "Freeze on Fridays.\n").await.status,
        StatusCode::OK
    );

    // The allow-list excuses a kind; `off` scans nothing.
    set_policy(&app, &w, "block", &["github-token"]).await;
    assert_eq!(
        create(&app, &w, &space, "Bot 4", &body).await.status,
        StatusCode::OK
    );
    set_policy(&app, &w, "off", &[]).await;
    assert_eq!(
        create(&app, &w, &space, "Bot 5", &body).await.status,
        StatusCode::OK
    );
    let off = scan(&app, &w, &body).await;
    assert_eq!(off["policy"], "off");
    assert_eq!(off["findings"], json!([]));

    let audit = audit_text(&app, &w).await;
    assert!(!audit.contains(&secret), "the audit log holds the secret");
    services.shutdown().await;
}

/// Uploads `text` as the file `name` on the world's page.
async fn upload(app: &Router, w: &World, name: &str, text: &str) -> Reply {
    let request = request(
        "POST",
        &format!("/api/v1/attachments/{}?name={name}", w.page_id),
    )
    .header(header::ORIGIN, ORIGIN)
    .header(header::COOKIE, &w.cookie)
    .header("x-csrf-token", &w.csrf)
    .header(header::CONTENT_TYPE, "text/plain")
    .header(header::CONTENT_LENGTH, text.len())
    .body(axum::body::Body::from(text.to_owned()))
    .unwrap();
    send(app, request).await
}

/// Asserts a typed refusal that names the kind, never the secret.
fn refused(reply: &Reply, secret: &str) {
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.text
    );
    assert!(reply.text.contains("github-token"), "{}", reply.text);
    assert!(!reply.text.contains(secret), "{}", reply.text);
}

#[tokio::test(flavor = "multi_thread")]
async fn text_uploads_are_scanned_too() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let secret = token();
    let note = format!("deploy bot: {secret}\n");

    // Warn: a text file holding a secret is kept, and audited. (Rows are
    // checked by vellum-collections' `RowCheck` tests and the guard's.)
    let kept = upload(&app, &w, "keys.txt", &note).await;
    assert_eq!(kept.status, StatusCode::CREATED, "{}", kept.text);
    let audit = audit_text(&app, &w).await;
    assert!(audit.contains("security.secret_found"), "{audit}");
    assert!(audit.contains(".attachments/keys.txt"), "{audit}");
    assert!(!audit.contains(&secret));

    // Block: the upload is refused with the same typed 422 as a page.
    set_policy(&app, &w, "block", &[]).await;
    refused(&upload(&app, &w, "keys-2.txt", &note).await, &secret);
    // Clean files still go through after a refusal.
    assert_eq!(
        upload(&app, &w, "notes.txt", "Freeze on Fridays.\n")
            .await
            .status,
        StatusCode::CREATED
    );
    services.shutdown().await;
}
