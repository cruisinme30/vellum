// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `/llms.txt`, `/sitemap.md` and `?format=md` inside `vellum serve`
//! (R-AI-6, D295): each caller sees only the pages they may read.

use axum::Router;
use http::{StatusCode, header};
use serde_json::json;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::identity_tests::{ORIGIN, Reply, cookie_of, finish, json_request, request, send};
use crate::mcp_tests::world;
use crate::services::{Services, VERSION};

/// What the stand-in web app answers for every path it gets.
const APP: &str = "the web app";

/// A server over a fresh plaintext workspace, with a stand-in web app.
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
    let web = Router::new().fallback(|| async { APP });
    let app = services
        .mount(GatewayBuilder::new(gateway, VERSION))
        .web_app(services.web_app(web))
        .build()
        .unwrap()
        .router();
    (services, app)
}

async fn get(app: &Router, path: &str, credential: Option<(header::HeaderName, &str)>) -> Reply {
    let mut b = request("GET", path);
    if let Some((name, value)) = credential {
        b = b.header(name, value);
    }
    send(app, finish(b)).await
}

fn content_type(reply: &Reply) -> &str {
    reply
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn models_and_crawlers_see_only_the_pages_they_may_read() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let page_md = format!("/p/{}?format=md", w.page_id);

    // Bo has an account but no place in the Engineering drive.
    let added = send(
        &app,
        w.post(
            "/api/v1/admin/users",
            &json!({"email": "bo@northstar.dev", "name": "Bo", "password": "battery horse staple"}),
        ),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
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
    let bearer = format!("Bearer {}", w.bearer);

    // The admin, by session or by token, sees the page in both lists,
    // linked to its Markdown.
    let llms = get(&app, "/llms.txt", Some((header::COOKIE, &w.cookie))).await;
    assert_eq!(llms.status, StatusCode::OK, "{}", llms.text);
    assert!(
        content_type(&llms).starts_with("text/plain"),
        "{:?}",
        llms.headers
    );
    assert!(llms.text.contains("[Runbook]"), "{}", llms.text);
    assert!(llms.text.contains(&page_md), "{}", llms.text);
    assert!(llms.text.contains("/sitemap.md"), "{}", llms.text);
    let sitemap = get(&app, "/sitemap.md", Some((header::AUTHORIZATION, &bearer))).await;
    assert_eq!(sitemap.status, StatusCode::OK, "{}", sitemap.text);
    assert!(content_type(&sitemap).starts_with("text/markdown"));
    assert!(sitemap.text.contains("## eng"), "{}", sitemap.text);
    assert!(sitemap.text.contains("[Runbook]"), "{}", sitemap.text);

    // Bo and anonymous visitors get the lists, without the page.
    for credential in [Some((header::COOKIE, bo.as_str())), None] {
        for path in ["/llms.txt", "/sitemap.md"] {
            let reply = get(&app, path, credential.clone()).await;
            assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.text);
            assert!(!reply.text.contains("Runbook"), "{path}: {}", reply.text);
        }
    }
    let anonymous = get(&app, "/llms.txt", None).await;
    assert!(
        anonymous
            .headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("public")),
        "{:?}",
        anonymous.headers
    );

    // The page as Markdown, for whoever may read it.
    for credential in [
        (header::COOKIE, w.cookie.as_str()),
        (header::AUTHORIZATION, &bearer),
    ] {
        let md = get(&app, &page_md, Some(credential)).await;
        assert_eq!(md.status, StatusCode::OK, "{}", md.text);
        assert!(
            content_type(&md).starts_with("text/markdown"),
            "{:?}",
            md.headers
        );
        assert!(md.text.contains("Freeze on Fridays."), "{}", md.text);
        assert!(
            md.headers
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("private")),
            "{:?}",
            md.headers
        );
    }
    // Anyone else gets the app, which asks them to sign in; the page's
    // text never leaves the server.
    for credential in [Some((header::COOKIE, bo.as_str())), None] {
        let reply = get(&app, &page_md, credential).await;
        assert_eq!(reply.text, APP);
    }
    // Without `?format=md`, the page URL is the app's as before.
    let html = get(
        &app,
        &format!("/p/{}", w.page_id),
        Some((header::COOKIE, &w.cookie)),
    )
    .await;
    assert_eq!(html.text, APP);
    services.shutdown().await;
}
