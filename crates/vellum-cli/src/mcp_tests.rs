// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `/mcp` inside `vellum serve`: a personal token, the real workspace, the
//! identity service's token check and the change feed (ADR 0019,
//! D217–D219).

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::identity_tests::{
    ORIGIN, Reply, csrf, finish, json_request, request, send, signed_in_admin,
};
use crate::services::{Services, VERSION};

fn mcp(token: Option<&str>, method: &str, params: &Value) -> Request<Body> {
    let mut b = request("POST", "/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(name) = params
        .get("name")
        .or_else(|| params.get("uri"))
        .and_then(Value::as_str)
    {
        b = b.header("mcp-name", name);
    }
    let body = vellum_mcp::http::request_body(1, method, params.clone());
    b.body(Body::from(body.to_string())).unwrap()
}

async fn tool(app: &Router, token: &str, name: &str, args: Value) -> Reply {
    send(
        app,
        mcp(
            Some(token),
            "tools/call",
            &json!({ "name": name, "arguments": args }),
        ),
    )
    .await
}

fn structured(reply: &Reply) -> Value {
    let body = reply.json();
    assert!(
        body["result"]["isError"] != json!(true),
        "tool failed: {}",
        reply.text
    );
    body["result"]["structuredContent"].clone()
}

/// A server over a fresh plaintext workspace.
async fn server(dir: &std::path::Path) -> (Services, Router) {
    let mut config = vellum_kernel::Config::default();
    config.data_dir = dir.to_path_buf();
    let services = Services::open(ORIGIN, Some(dir), None)
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
    (services, app)
}

/// A signed-in admin with a shared drive `eng` holding one page, and a
/// personal token for MCP: the session cookie, its CSRF token, the page's
/// ID and the bearer token.
struct World {
    space_id: String,
    cookie: String,
    csrf: String,
    page_id: String,
    bearer: String,
}

impl World {
    fn post(&self, path: &str, body: &Value) -> Request<Body> {
        finish(
            json_request("POST", path, body)
                .header(header::COOKIE, &self.cookie)
                .header("x-csrf-token", &self.csrf),
        )
    }
}

async fn world(app: &Router) -> World {
    let cookie = signed_in_admin(app).await;
    let mut w = World {
        csrf: csrf(app, &cookie).await,
        cookie,
        space_id: String::new(),
        page_id: String::new(),
        bearer: String::new(),
    };
    let space = send(
        app,
        w.post(
            "/api/v1/drive/spaces",
            &json!({"kind": "shared_drive", "slug": "eng", "name": "Engineering"}),
        ),
    )
    .await;
    assert_eq!(space.status, StatusCode::OK, "{}", space.text);
    w.space_id = space.json()["id"].as_str().unwrap().to_owned();
    let space_id = w.space_id.clone();
    let page = send(
        app,
        w.post(
            &format!("/api/v1/drive/spaces/{space_id}/pages"),
            &json!({"title": "Runbook", "body": "## Deploys\n\nFreeze on Fridays.\n"}),
        ),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    w.page_id = page.json()["id"].as_str().unwrap().to_owned();
    let minted = send(
        app,
        w.post(
            "/api/v1/me/tokens",
            &json!({"name": "Claude", "scopes": ["read", "write:draft", "work"]}),
        ),
    )
    .await;
    assert_eq!(minted.status, StatusCode::OK, "{}", minted.text);
    w.bearer = minted.json()["token"].as_str().unwrap().to_owned();
    w
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_is_a_resource_server_of_the_identity_service() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    // Without a token, and with a bad one, the endpoint itself answers
    // with the challenge that points clients at the metadata.
    for token in [None, Some("vlm_not_a_real_token")] {
        let reply = send(&app, mcp(token, "tools/list", &json!({}))).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text);
        let challenge = reply
            .headers
            .get(header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(challenge.contains("resource_metadata="), "{challenge}");
    }
    let metadata = send(
        &app,
        finish(request("GET", "/.well-known/oauth-protected-resource/mcp")),
    )
    .await;
    assert_eq!(metadata.status, StatusCode::OK, "{}", metadata.text);
    assert_eq!(metadata.json()["resource"], format!("{ORIGIN}/mcp"));
    let issuer = send(
        &app,
        finish(request("GET", "/.well-known/oauth-authorization-server")),
    )
    .await;
    assert_eq!(issuer.status, StatusCode::OK, "identity's metadata stays");

    let w = world(&app).await;
    let listed = send(&app, mcp(Some(&w.bearer), "tools/list", &json!({}))).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    // Direct writes need write:direct, which this token lacks.
    let direct = tool(
        &app,
        &w.bearer,
        "propose_edit",
        json!({ "doc_id": w.page_id, "summary": "x", "direct": true, "edits": [] }),
    )
    .await;
    assert_eq!(direct.status, StatusCode::FORBIDDEN, "{}", direct.text);
    services.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_personal_token_reads_and_drafts_through_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let (page_id, bearer) = (&w.page_id, &w.bearer);

    // Reads go through the drive as the token's user.
    let doc = structured(&tool(&app, bearer, "get_doc", json!({ "id": page_id })).await);
    assert_eq!(doc["title"], "Runbook");
    assert!(
        doc["markdown"]
            .as_str()
            .unwrap()
            .contains("Freeze on Fridays.")
    );
    let version = doc["version"].as_str().unwrap().to_owned();
    let blocks = structured(&tool(&app, bearer, "get_blocks", json!({ "doc_id": page_id })).await);
    let freeze = blocks["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| {
            b["markdown"]
                .as_str()
                .unwrap_or_default()
                .contains("Fridays")
        })
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let tree = send(
        &app,
        mcp(
            Some(bearer),
            "resources/read",
            &json!({ "uri": "vellum://space/eng/tree" }),
        ),
    )
    .await;
    assert!(tree.text.contains("Runbook"), "{}", tree.text);

    // A proposed edit is a draft in My Drive; the page is unchanged.
    let edit = structured(
        &tool(
            &app,
            bearer,
            "propose_edit",
            json!({
                "doc_id": page_id,
                "base_version": version,
                "summary": "Allow hotfixes",
                "edits": [{ "op": "replace", "block_id": freeze, "markdown": "Freeze on Fridays, except hotfixes." }],
            }),
        )
        .await,
    );
    assert_eq!(edit["mode"], "draft", "{edit}");
    let draft = structured(&tool(&app, bearer, "get_doc", json!({ "id": edit["id"] })).await);
    let draft_md = draft["markdown"].as_str().unwrap();
    assert!(draft_md.contains("status: draft"), "{draft_md}");
    assert!(
        draft_md.contains(&format!("draft_of: {page_id}")),
        "{draft_md}"
    );
    assert!(draft_md.contains("except hotfixes"), "{draft_md}");
    let again = structured(&tool(&app, bearer, "get_doc", json!({ "id": page_id })).await);
    assert_eq!(again["version"], version, "the page itself is untouched");

    // A new page lands where asked, as a draft with its attribution.
    let created = structured(
        &tool(
            &app,
            bearer,
            "create_draft",
            json!({ "parent": "space:eng", "title": "Postmortem", "markdown": "What happened." }),
        )
        .await,
    );
    let new_page = structured(&tool(&app, bearer, "get_doc", json!({ "id": created["id"] })).await);
    let md = new_page["markdown"].as_str().unwrap();
    assert!(
        md.contains("status: draft") && md.contains("authored_by: model"),
        "{md}"
    );
    services.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listeners_hear_about_changes_from_the_feed() {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let uri = format!("vellum://doc/{}", w.page_id);
    let listen = app
        .clone()
        .oneshot(mcp(
            Some(&w.bearer),
            "subscriptions/listen",
            &json!({ "notifications": { "resourceSubscriptions": [uri] } }),
        ))
        .await
        .unwrap();
    assert_eq!(listen.status(), StatusCode::OK);
    let mut stream = listen.into_body();
    let ack = tokio::time::timeout(std::time::Duration::from_secs(5), stream.frame()).await;
    let mut seen = format!("{ack:?}");
    assert!(seen.contains("acknowledged"), "{seen}");

    // An edit through the REST API, as the web app makes it.
    let read = send(
        &app,
        finish(
            request("GET", &format!("/api/v1/drive/items/{}/content", w.page_id))
                .header(header::COOKIE, &w.cookie),
        ),
    )
    .await;
    let current = read.json();
    let saved = send(
        &app,
        finish(
            json_request(
                "PUT",
                &format!("/api/v1/drive/items/{}/content", w.page_id),
                &json!({
                    "id": w.page_id,
                    "base_content": current["content"],
                    "base_etag": current["etag"],
                    "content": format!("{}\nThaw on Mondays.\n", current["content"].as_str().unwrap_or_default()),
                }),
            )
            .header(header::COOKIE, &w.cookie)
            .header("x-csrf-token", &w.csrf),
        ),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !seen.contains("notifications/resources/updated")
        && tokio::time::Instant::now() < deadline
    {
        if let Ok(Some(Ok(frame))) =
            tokio::time::timeout(std::time::Duration::from_secs(1), stream.frame()).await
            && let Ok(data) = frame.into_data()
        {
            seen.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(seen.contains("notifications/resources/updated"), "{seen}");
    assert!(seen.contains(&uri), "{seen}");
    services.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn page_templates_come_from_templates_folders() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let folder = send(
        &app,
        w.post(
            &format!("/api/v1/drive/spaces/{}/folders", w.space_id),
            &json!({ "name": "Templates" }),
        ),
    )
    .await;
    assert_eq!(folder.status, StatusCode::OK, "{}", folder.text);
    let page = send(
        &app,
        w.post(
            &format!("/api/v1/drive/spaces/{}/pages", w.space_id),
            &json!({
                "parent_id": folder.json()["id"],
                "title": "PRD",
                "body": "## Problem\n\n## Requirements\n",
            }),
        ),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);

    let list = structured(&tool(&app, &w.bearer, "get_template", json!({})).await);
    assert_eq!(list["templates"][0]["name"], "prd", "{list}");
    assert_eq!(list["templates"].as_array().unwrap().len(), 1, "{list}");
    let prd = structured(&tool(&app, &w.bearer, "get_template", json!({ "name": "PRD" })).await);
    let md = prd["markdown"].as_str().unwrap();
    assert!(
        md.contains("## Requirements") && !md.contains("id:"),
        "{md}"
    );
    let missing = tool(&app, &w.bearer, "get_template", json!({ "name": "nope" })).await;
    assert_eq!(
        missing.json()["result"]["isError"],
        true,
        "{}",
        missing.text
    );
    services.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn export_hands_over_the_file_and_its_route() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    // The tool runs the export routes' own code, as the token's user.
    let made = tool(
        &app,
        &w.bearer,
        "export",
        json!({ "target": w.page_id, "format": "pdf" }),
    )
    .await;
    let body = made.json();
    let a = &body["result"]["structuredContent"];
    assert_eq!(a["mime_type"], "application/pdf", "{}", made.text);
    assert_eq!(a["name"], "Runbook.pdf", "{a}");
    assert!(a["size"].as_u64().unwrap() > 0, "{a}");
    assert_eq!(
        a["url"],
        format!("{ORIGIN}/api/v1/export/pages/{}?format=pdf", w.page_id)
    );
    assert!(a.get("data").is_none(), "the file is not repeated: {a}");
    let blob = body["result"]["content"][2]["resource"]["blob"]
        .as_str()
        .unwrap();
    assert!(blob.starts_with("JVBERi0"), "a PDF: {}", made.text);

    // The same file comes from the route the result links.
    let route = send(
        &app,
        Request::get(format!("/api/v1/export/pages/{}?format=pdf", w.page_id))
            .header(header::AUTHORIZATION, format!("Bearer {}", w.bearer))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(route.status, StatusCode::OK, "{}", route.text);

    let missing = tool(
        &app,
        &w.bearer,
        "export",
        json!({ "target": "01JBQ8Y4ZK6V3W2N5R7T9X1C41", "format": "docx", "section": "s1" }),
    )
    .await;
    assert_eq!(
        missing.json()["result"]["isError"],
        true,
        "{}",
        missing.text
    );
    services.shutdown().await;
}
