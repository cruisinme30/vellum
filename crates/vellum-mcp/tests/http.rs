// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Contract tests of the Streamable HTTP transport against MCP 2026-07-28.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vellum_mcp::auth::{ScopeSet, VerifiedToken};
use vellum_mcp::http::{HttpConfig, McpHttp, request_body, router};
use vellum_mcp::memory::{MemoryAudit, MemoryWorkspace, StaticTokens};
use vellum_mcp::ports::{Actor, Workspace, WriteMode};
use vellum_mcp::ratelimit::RateLimit;
use vellum_mcp::server::{McpServer, Notifier, ServerConfig};
use vellum_mcp::site;

const BASE: &str = "https://vellum.northstar.dev";

fn token(id: &str, scopes: &str, audience: &str) -> VerifiedToken {
    VerifiedToken {
        token_id: id.into(),
        user_id: "usr_priya".into(),
        user_name: "Priya".into(),
        client_id: "https://claude.ai/oauth/mcp-client.json".into(),
        client_name: "Claude".into(),
        scopes: ScopeSet::parse(scopes),
        audience: vec![audience.into()],
        expires_at: None,
    }
}

struct Harness {
    app: Router,
    ws: Arc<MemoryWorkspace>,
    audit: Arc<MemoryAudit>,
    notifier: Notifier,
}

fn harness_with(limit: RateLimit) -> Harness {
    let ws = Arc::new(MemoryWorkspace::northstar());
    let audit = Arc::new(MemoryAudit::default());
    let aud = format!("{BASE}/mcp");
    let tokens = StaticTokens::default()
        .with("read", token("tok_read", "read", &aud))
        .with("draft", token("tok_draft", "write:draft work", &aud))
        .with("direct", token("tok_direct", "write:direct", &aud))
        .with("work", token("tok_work", "work", &aud))
        .with(
            "elsewhere",
            token("tok_else", "read", "https://other.example/mcp"),
        );
    let server = Arc::new(McpServer::new(
        ws.clone(),
        audit.clone(),
        ServerConfig {
            rate_limit: limit,
            ..ServerConfig::default()
        },
    ));
    let notifier = server.notifier();
    let mut config = HttpConfig::new(BASE);
    config.keep_alive = Duration::from_millis(50);
    let state = Arc::new(McpHttp {
        server,
        verifier: Arc::new(tokens),
        config,
    });
    Harness {
        app: router(state),
        ws,
        audit,
        notifier,
    }
}

fn harness() -> Harness {
    harness_with(RateLimit {
        burst: 1000,
        per_minute: 1000,
    })
}

fn post(token: Option<&str>, method: &str, name: Option<&str>, body: &Value) -> Request<Body> {
    let mut b = Request::post("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(n) = name {
        b = b.header("mcp-name", n);
    }
    b.body(Body::from(body.to_string())).unwrap_or_default()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let resp = app.clone().oneshot(req).await.unwrap_or_default();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .unwrap_or_default();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, json)
}

async fn rpc(
    h: &Harness,
    token: &str,
    method: &str,
    params: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let name = params
        .get("name")
        .or_else(|| params.get("uri"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    send(
        &h.app,
        post(
            Some(token),
            method,
            name.as_deref(),
            &request_body(7, method, params),
        ),
    )
    .await
}

async fn call_tool(
    h: &Harness,
    token: &str,
    tool: &str,
    args: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    rpc(
        h,
        token,
        "tools/call",
        json!({ "name": tool, "arguments": args }),
    )
    .await
}

#[tokio::test]
async fn requests_without_a_token_get_an_oauth_challenge() {
    let h = harness();
    let (status, headers, body) = send(
        &h.app,
        post(
            None,
            "server/discover",
            None,
            &request_body(1, "server/discover", json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(challenge.starts_with("Bearer "));
    assert!(challenge.contains(&format!(
        "resource_metadata=\"{BASE}/.well-known/oauth-protected-resource/mcp\""
    )));
    assert!(body.get("error").is_some());
}

#[tokio::test]
async fn tokens_for_another_audience_are_refused() {
    let h = harness();
    let (status, headers, _) = rpc(&h, "elsewhere", "server/discover", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(challenge.contains("error=\"invalid_token\""));
}

#[tokio::test]
async fn publishes_protected_resource_metadata() {
    let h = harness();
    let req = Request::get("/.well-known/oauth-protected-resource/mcp")
        .body(Body::empty())
        .unwrap_or_default();
    let (status, _, body) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resource"], format!("{BASE}/mcp"));
    assert_eq!(body["authorization_servers"][0], BASE);
    assert_eq!(body["bearer_methods_supported"], json!(["header"]));
}

#[tokio::test]
async fn get_and_delete_are_not_allowed() {
    let h = harness();
    for method in ["GET", "DELETE"] {
        let req = Request::builder()
            .method(method)
            .uri("/mcp")
            .body(Body::empty())
            .unwrap_or_default();
        let (status, headers, _) = send(&h.app, req).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            headers.get(header::ALLOW).and_then(|v| v.to_str().ok()),
            Some("POST")
        );
    }
}

#[tokio::test]
async fn discover_reports_versions_capabilities_and_cache_hints() {
    let h = harness();
    let (status, headers, body) = rpc(&h, "read", "server/discover", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.contains_key("x-request-id"));
    assert!(headers.contains_key("server-timing"));
    let r = &body["result"];
    assert_eq!(r["resultType"], "complete");
    assert_eq!(r["supportedVersions"], json!(["2026-07-28", "2025-11-25"]));
    assert_eq!(r["capabilities"]["resources"]["subscribe"], true);
    assert_eq!(r["cacheScope"], "public");
    assert!(r["ttlMs"].as_u64().is_some());
    assert_eq!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "vellum"
    );
    assert!(
        r["instructions"]
            .as_str()
            .is_some_and(|s| s.contains("Draft"))
    );
}

#[tokio::test]
async fn missing_meta_is_invalid_params() {
    let h = harness();
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let (status, _, resp) = send(&h.app, post(Some("read"), "tools/list", None, &body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn unsupported_versions_list_what_is_supported() {
    let h = harness();
    let mut req = post(
        Some("read"),
        "tools/list",
        None,
        &request_body(1, "tools/list", json!({})),
    );
    let _ = req.headers_mut().insert(
        "mcp-protocol-version",
        axum::http::HeaderValue::from_static("2099-01-01"),
    );
    let (status, _, resp) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], -32022);
    assert_eq!(resp["error"]["data"]["supported"][0], "2026-07-28");
}

#[tokio::test]
async fn mirrored_headers_must_match_the_body() {
    let h = harness();
    let body = request_body(
        1,
        "tools/call",
        json!({ "name": "search", "arguments": { "query": "plan" } }),
    );
    let (status, _, resp) = send(
        &h.app,
        post(Some("read"), "tools/list", Some("search"), &body),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], -32020);
    let (status, _, resp) = send(
        &h.app,
        post(Some("read"), "tools/call", Some("get_doc"), &body),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], -32020);
    let (status, _, _) = send(&h.app, post(Some("read"), "tools/call", None, &body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Base64-encoded Mcp-Name is decoded before comparison.
    let (status, _, resp) = send(
        &h.app,
        post(
            Some("read"),
            "tools/call",
            Some("=?base64?c2VhcmNo?="),
            &body,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
}

#[tokio::test]
async fn foreign_origins_are_rejected() {
    let h = harness();
    let mut req = post(
        Some("read"),
        "server/discover",
        None,
        &request_body(1, "server/discover", json!({})),
    );
    let _ = req.headers_mut().insert(
        header::ORIGIN,
        axum::http::HeaderValue::from_static("https://evil.example"),
    );
    let (status, _, _) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tools_list_is_deterministic_and_public() {
    let h = harness();
    let (_, _, a) = rpc(&h, "read", "tools/list", json!({})).await;
    let (_, _, b) = rpc(&h, "direct", "tools/list", json!({})).await;
    assert_eq!(
        a["result"]["tools"], b["result"]["tools"],
        "same list for every token"
    );
    let names: Vec<&str> = a["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            "search",
            "get_doc",
            "get_blocks",
            "get_outline",
            "get_template",
            "create_draft",
            "propose_edit",
            "create_issue",
            "update_issue",
            "add_comment",
            "create_task",
            "query_collection",
            "upsert_rows",
            "export",
            "list_changes"
        ]
    );
    assert_eq!(a["result"]["cacheScope"], "public");
}

#[tokio::test]
async fn search_is_permission_filtered_and_cites_blocks() {
    let h = harness();
    let (status, _, body) = call_tool(&h, "read", "search", json!({ "query": "salary" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["result"]["structuredContent"]["hits"],
        json!([]),
        "Priya cannot read salary bands"
    );
    let (_, _, body) = call_tool(&h, "read", "search", json!({ "query": "renderer" })).await;
    let hit = &body["result"]["structuredContent"]["hits"][0];
    assert_eq!(hit["block_id"], "blk-pdfr0001");
    assert_eq!(hit["heading_path"], json!(["Risks"]));
    assert!(
        hit["citation_url"]
            .as_str()
            .is_some_and(|u| u.ends_with("#blk-pdfr0001"))
    );
}

#[tokio::test]
async fn reads_flag_instruction_like_blocks() {
    let h = harness();
    let (_, _, body) = call_tool(
        &h,
        "read",
        "get_doc",
        json!({ "id": "01JDINJECT0000000000000000" }),
    )
    .await;
    let warnings = &body["result"]["structuredContent"]["warnings"];
    assert_eq!(warnings[0]["kind"], "instruction_like");
    assert_eq!(warnings[0]["block_id"], "blk-evil0001");
}

#[tokio::test]
async fn unreadable_documents_are_tool_errors_not_leaks() {
    let h = harness();
    let (status, _, body) = call_tool(
        &h,
        "read",
        "get_doc",
        json!({ "id": "01JDSECRET0000000000000000" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(!body.to_string().contains("Confidential"));
}

#[tokio::test]
async fn model_writes_are_drafts_attributed_to_the_model() {
    let h = harness();
    let args = json!({ "parent": "space:eng", "title": "Q4 plan", "markdown": "Draft body." });
    let (status, _, body) = call_tool(&h, "draft", "create_draft", args.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["structuredContent"]["mode"], "draft");
    assert!(
        body["result"]["structuredContent"]["url"]
            .as_str()
            .is_some_and(|u| u.contains("/review/"))
    );
    let writes = h.ws.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes.first().map(|w| w.mode), Some(WriteMode::Draft));
    assert_eq!(
        writes.first().map(|w| w.by.authored_by.as_str()),
        Some("model")
    );
    assert_eq!(
        writes.first().map(|w| w.by.on_behalf_of.as_str()),
        Some("usr_priya")
    );
}

#[tokio::test]
async fn direct_writes_need_write_direct() {
    let h = harness();
    let args =
        json!({ "parent": "space:eng", "title": "Q4 plan", "markdown": "Body.", "direct": true });
    let (status, headers, body) = call_tool(&h, "draft", "create_draft", args.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let challenge = headers
        .get(header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(challenge.contains("error=\"insufficient_scope\""));
    assert!(challenge.contains("scope=\"write:direct\""));
    assert_eq!(
        body["error"]["data"]["requiredScopes"],
        json!(["write:direct"])
    );
    let (status, _, body) = call_tool(&h, "direct", "create_draft", args).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["structuredContent"]["mode"], "direct");
}

#[tokio::test]
async fn read_only_tokens_cannot_write() {
    let h = harness();
    let (status, _, _) = call_tool(
        &h,
        "read",
        "create_issue",
        json!({ "team": "ENG", "title": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) = call_tool(
        &h,
        "work",
        "create_issue",
        json!({ "team": "ENG", "title": "Export fails" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["structuredContent"]["state"], "Triage");
}

#[tokio::test]
async fn stale_edits_are_reported_to_the_model() {
    let h = harness();
    let args = json!({
        "doc_id": "01JDLAUNCH0000000000000000", "base_version": "v0", "summary": "fix date",
        "edits": [{ "op": "replace", "block_id": "blk-ship0001", "markdown": "Ship on 2026-12-01." }]
    });
    let (_, _, body) = call_tool(&h, "draft", "propose_edit", args).await;
    assert_eq!(body["result"]["isError"], true);
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("conflict"))
    );
}

#[tokio::test]
async fn bad_arguments_and_unknown_tools() {
    let h = harness();
    let (_, _, body) = call_tool(&h, "read", "get_doc", json!({ "nope": 1 })).await;
    assert_eq!(body["result"]["isError"], true);
    let (_, _, body) = call_tool(&h, "read", "drop_tables", json!({})).await;
    assert_eq!(body["error"]["code"], -32602);
}

#[tokio::test]
async fn export_returns_a_resource_link() {
    let h = harness();
    let (_, _, body) = call_tool(
        &h,
        "read",
        "export",
        json!({ "target": "01JDLAUNCH0000000000000000", "format": "pdf" }),
    )
    .await;
    assert_eq!(body["result"]["content"][1]["type"], "resource_link");
    assert_eq!(body["result"]["content"][1]["mimeType"], "application/pdf");
}

#[tokio::test]
async fn resources_read_lists_and_templates() {
    let h = harness();
    let (_, _, body) = rpc(
        &h,
        "read",
        "resources/read",
        json!({ "uri": "vellum://doc/01JDLAUNCH0000000000000000" }),
    )
    .await;
    let r = &body["result"];
    assert_eq!(r["cacheScope"], "private");
    assert_eq!(r["contents"][0]["mimeType"], "text/markdown");
    assert!(
        r["contents"][0]["text"]
            .as_str()
            .is_some_and(|t| t.starts_with("---\nid: 01JDLAUNCH"))
    );
    let (_, _, body) = rpc(
        &h,
        "read",
        "resources/read",
        json!({ "uri": "vellum://doc/01JDSECRET0000000000000000" }),
    )
    .await;
    assert_eq!(body["error"]["code"], -32602);
    let (_, _, body) = rpc(
        &h,
        "read",
        "resources/read",
        json!({ "uri": "vellum://issue/ENG-142" }),
    )
    .await;
    assert!(
        body["result"]["contents"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("number: 142"))
    );
    let (_, _, body) = rpc(&h, "read", "resources/list", json!({})).await;
    let uris: Vec<&str> = body["result"]["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["uri"].as_str())
        .collect();
    assert!(uris.contains(&"vellum://doc/01JDLAUNCH0000000000000000"));
    assert!(!uris.contains(&"vellum://doc/01JDSECRET0000000000000000"));
    let (_, _, body) = rpc(&h, "read", "resources/templates/list", json!({})).await;
    assert_eq!(
        body["result"]["resourceTemplates"].as_array().map(Vec::len),
        Some(5)
    );
}

#[tokio::test]
async fn prompts_embed_guarded_resources() {
    let h = harness();
    let (_, _, body) = rpc(&h, "read", "prompts/list", json!({})).await;
    assert_eq!(body["result"]["prompts"].as_array().map(Vec::len), Some(5));
    let (_, _, body) = rpc(
        &h,
        "read",
        "prompts/get",
        json!({ "name": "verify-page", "arguments": { "doc_id": "01JDLAUNCH0000000000000000" } }),
    )
    .await;
    let msgs = &body["result"]["messages"];
    assert!(
        msgs[0]["content"]["text"]
            .as_str()
            .is_some_and(|t| t.contains("propose_edit"))
    );
    assert_eq!(
        msgs[1]["content"]["resource"]["uri"],
        "vellum://doc/01JDLAUNCH0000000000000000"
    );
    let (_, _, body) = rpc(
        &h,
        "read",
        "prompts/get",
        json!({ "name": "triage-issue", "arguments": {} }),
    )
    .await;
    assert_eq!(body["error"]["code"], -32602);
}

#[tokio::test]
async fn rate_limits_apply_per_token() {
    let h = harness_with(RateLimit {
        burst: 2,
        per_minute: 1,
    });
    for _ in 0..2 {
        let (status, _, _) = rpc(&h, "read", "tools/list", json!({})).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, headers, body) = rpc(&h, "read", "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key(header::RETRY_AFTER));
    assert_eq!(body["error"]["code"], -31029);
    let (status, _, _) = rpc(&h, "draft", "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "another token has its own bucket");
    assert!(h.audit.events().iter().any(|e| e.outcome == "rate_limited"));
}

#[tokio::test]
async fn every_call_is_audited_with_its_trace() {
    let h = harness();
    let mut body = request_body(
        9,
        "tools/call",
        json!({ "name": "get_doc", "arguments": { "id": "01JDLAUNCH0000000000000000" } }),
    );
    body["params"]["_meta"]["traceparent"] =
        json!("00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01");
    let _ = send(
        &h.app,
        post(Some("read"), "tools/call", Some("get_doc"), &body),
    )
    .await;
    let _ = call_tool(&h, "read", "create_task", json!({ "title": "x" })).await;
    let events = h.audit.events();
    let read = events
        .iter()
        .find(|e| e.target.as_deref() == Some("get_doc"));
    assert_eq!(
        read.map(|e| e.touched.clone()),
        Some(vec!["01JDLAUNCH0000000000000000".to_owned()])
    );
    assert_eq!(
        read.and_then(|e| e.trace_id.clone()).as_deref(),
        Some("0af7651916cd43dd8448eb211c80319c")
    );
    assert_eq!(
        read.map(|e| e.client_id.as_str()),
        Some("https://claude.ai/oauth/mcp-client.json")
    );
    assert!(
        events
            .iter()
            .any(|e| e.target.as_deref() == Some("create_task") && e.outcome == "denied")
    );
}

#[tokio::test]
async fn legacy_clients_can_initialize_and_call_without_meta() {
    let h = harness();
    let init = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "old", "version": "1" } } });
    let req = Request::post("/mcp")
        .header(header::AUTHORIZATION, "Bearer read")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(init.to_string()))
        .unwrap_or_default();
    let (status, _, body) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    let req = Request::post("/mcp")
        .header(header::AUTHORIZATION, "Bearer read")
        .header("mcp-protocol-version", "2025-11-25")
        .body(Body::from(list.to_string()))
        .unwrap_or_default();
    let (status, _, body) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["tools"].as_array().map(Vec::len), Some(15));
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let req = Request::post("/mcp")
        .header(header::AUTHORIZATION, "Bearer read")
        .header("mcp-protocol-version", "2025-11-25")
        .body(Body::from(note.to_string()))
        .unwrap_or_default();
    let (status, _, _) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn subscriptions_listen_streams_acknowledged_updates() {
    let h = harness();
    let body = request_body(
        42,
        "subscriptions/listen",
        json!({ "notifications": {
        "toolsListChanged": true,
        "resourceSubscriptions": ["vellum://doc/01JDLAUNCH0000000000000000", "vellum://doc/01JDSECRET0000000000000000"]
    } }),
    );
    let resp = h
        .app
        .clone()
        .oneshot(post(Some("read"), "subscriptions/listen", None, &body))
        .await
        .unwrap_or_default();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(
        resp.headers()
            .get("x-accel-buffering")
            .and_then(|v| v.to_str().ok()),
        Some("no")
    );
    let mut stream = resp.into_body();
    let mut next = async || -> String {
        let frame = tokio::time::timeout(Duration::from_secs(2), stream.frame()).await;
        frame
            .ok()
            .flatten()
            .and_then(Result::ok)
            .and_then(|f| f.into_data().ok())
            .map(|d| String::from_utf8_lossy(&d).into_owned())
            .unwrap_or_default()
    };
    let ack = next().await;
    assert!(
        ack.contains("notifications/subscriptions/acknowledged"),
        "{ack}"
    );
    assert!(ack.contains("\"io.modelcontextprotocol/subscriptionId\":42"));
    assert!(ack.contains("01JDLAUNCH"));
    assert!(
        !ack.contains("01JDSECRET"),
        "unreadable resources are not honoured"
    );
    assert!(
        !ack.contains("toolsListChanged"),
        "the tool list never changes"
    );
    h.notifier
        .resource_updated("vellum://doc/01JDSECRET0000000000000000");
    h.notifier
        .resource_updated("vellum://doc/01JDLAUNCH0000000000000000");
    let mut update = next().await;
    while update.starts_with(':') {
        update = next().await;
    }
    assert!(
        update.contains("notifications/resources/updated"),
        "{update}"
    );
    assert!(update.contains("01JDLAUNCH"));
}

#[tokio::test]
async fn test_connection_and_snippets() {
    let h = harness();
    let req = Request::post("/mcp/test-connection")
        .header(header::AUTHORIZATION, "Bearer read")
        .body(Body::empty())
        .unwrap_or_default();
    let (status, _, report) = send(&h.app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["checks"].as_array().map(Vec::len), Some(4));
    let req = Request::post("/mcp/test-connection")
        .header(header::AUTHORIZATION, "Bearer elsewhere")
        .body(Body::empty())
        .unwrap_or_default();
    let (_, _, report) = send(&h.app, req).await;
    assert_eq!(report["ok"], false);
    let req = Request::get("/mcp/setup")
        .body(Body::empty())
        .unwrap_or_default();
    let (_, _, body) = send(&h.app, req).await;
    assert_eq!(body["snippets"].as_array().map(Vec::len), Some(4));
}

#[tokio::test]
async fn llms_txt_sitemap_and_format_md() {
    let ws: Arc<dyn Workspace> = Arc::new(MemoryWorkspace::northstar());
    let app = site::router(ws.clone())
        .route(
            "/eng/launch-plan",
            axum::routing::get(|| async { "<html>page</html>" }),
        )
        .route(
            "/eng/vendor-notes",
            axum::routing::get(|| async { "<html>page</html>" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            ws.clone(),
            site::format_md,
        ));
    let req = Request::get("/llms.txt")
        .body(Body::empty())
        .unwrap_or_default();
    let resp = app.clone().oneshot(req).await.unwrap_or_default();
    let text = String::from_utf8_lossy(
        &resp
            .into_body()
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default(),
    )
    .into_owned();
    assert!(text.starts_with("# Northstar\n\n> "));
    assert!(text.contains("[Launch plan](https://vellum.northstar.dev/eng/launch-plan?format=md)"));
    assert!(
        !text.contains("Vendor notes"),
        "anonymous visitors see public pages only"
    );
    let req = Request::get("/sitemap.md")
        .body(Body::empty())
        .unwrap_or_default();
    let resp = app.clone().oneshot(req).await.unwrap_or_default();
    assert_eq!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/markdown; charset=utf-8")
    );
    let req = Request::get("/eng/launch-plan?format=md")
        .body(Body::empty())
        .unwrap_or_default();
    let resp = app.clone().oneshot(req).await.unwrap_or_default();
    let md = String::from_utf8_lossy(
        &resp
            .into_body()
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default(),
    )
    .into_owned();
    assert!(md.starts_with("---\nid: 01JDLAUNCH"));
    let req = Request::get("/eng/launch-plan")
        .body(Body::empty())
        .unwrap_or_default();
    let resp = app.clone().oneshot(req).await.unwrap_or_default();
    let html = String::from_utf8_lossy(
        &resp
            .into_body()
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default(),
    )
    .into_owned();
    assert_eq!(html, "<html>page</html>");
    let _ = Actor::Anonymous;
}

#[tokio::test]
async fn templates_are_listed_and_read() {
    let h = harness();
    let (status, _, list) = call_tool(&h, "read", "get_template", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["result"]["structuredContent"]["templates"][0]["name"],
        "prd"
    );
    let (_, _, prd) = call_tool(&h, "read", "get_template", json!({ "name": "prd" })).await;
    let md = prd["result"]["structuredContent"]["markdown"]
        .as_str()
        .unwrap_or_default();
    assert!(md.contains("## Requirements"), "{prd}");
    let (_, _, missing) = call_tool(&h, "read", "get_template", json!({ "name": "nope" })).await;
    assert_eq!(missing["result"]["isError"], true, "{missing}");
}

/// A token that can be revoked mid-test.
struct Revocable {
    token: VerifiedToken,
    revoked: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl vellum_mcp::auth::TokenVerifier for Revocable {
    async fn verify(&self, _token: &str) -> Result<VerifiedToken, vellum_mcp::auth::TokenError> {
        if self.revoked.load(std::sync::atomic::Ordering::SeqCst) {
            Err(vellum_mcp::auth::TokenError::Invalid)
        } else {
            Ok(self.token.clone())
        }
    }
}

/// RT-047 (#552): a listen stream ends once its token stops verifying,
/// instead of reporting changes for ever.
#[tokio::test]
async fn listen_streams_close_when_the_token_is_revoked() {
    let ws = Arc::new(MemoryWorkspace::northstar());
    let server = Arc::new(McpServer::new(
        ws,
        Arc::new(MemoryAudit::default()),
        ServerConfig::default(),
    ));
    let notifier = server.notifier();
    let verifier = Arc::new(Revocable {
        token: token("tok_read", "read", &format!("{BASE}/mcp")),
        revoked: std::sync::atomic::AtomicBool::new(false),
    });
    let mut config = HttpConfig::new(BASE);
    config.keep_alive = Duration::from_millis(50);
    let app = router(Arc::new(McpHttp {
        server,
        verifier: verifier.clone(),
        config,
    }));
    let body = request_body(
        7,
        "subscriptions/listen",
        json!({ "notifications": {
            "resourceSubscriptions": ["vellum://doc/01JDLAUNCH0000000000000000"]
        } }),
    );
    let resp = app
        .oneshot(post(Some("read"), "subscriptions/listen", None, &body))
        .await
        .unwrap_or_default();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut stream = resp.into_body();
    let mut next = async || -> Option<String> {
        let frame = tokio::time::timeout(Duration::from_secs(2), stream.frame())
            .await
            .ok()??;
        frame
            .ok()
            .and_then(|f| f.into_data().ok())
            .map(|d| String::from_utf8_lossy(&d).into_owned())
    };
    let ack = next().await.unwrap_or_default();
    assert!(ack.contains("acknowledged"), "{ack}");

    verifier
        .revoked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    notifier.resource_updated("vellum://doc/01JDLAUNCH0000000000000000");
    let mut seen = Vec::new();
    while let Some(frame) = next().await {
        seen.push(frame);
    }
    assert!(
        !seen.iter().any(|f| f.contains("resources/updated")),
        "no update after revocation: {seen:?}"
    );
    assert!(
        seen.last().is_some_and(|f| f.contains("\"id\":7")),
        "the stream ends with its closing result: {seen:?}"
    );
}
