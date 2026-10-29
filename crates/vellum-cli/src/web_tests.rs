// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The web app through the real gateway.

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt as _;
use tower::ServiceExt as _;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::services::{Services, VERSION};
use crate::{serve, web};

const ORIGIN: &str = "http://localhost";
const INDEX: &[u8] = b"<!doctype html><title>Vellum</title><div id=root></div>";

static FILES: &[(&str, &[u8])] = &[
    ("/index.html", INDEX),
    ("/assets/index-3f2a9c.js", b"console.log(1)"),
    ("/assets/inter-latin.woff2", b"wOF2"),
    ("/sw.js", b"self.addEventListener('fetch', () => {})"),
    ("/manifest.webmanifest", b"{}"),
];

struct Reply {
    status: StatusCode,
    headers: http::HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: header::HeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }

    fn is_index(&self) -> bool {
        self.body == INDEX
    }
}

fn gateway(public_pages: bool, files: Option<&'static [(&'static str, &'static [u8])]>) -> Router {
    let services = Services::open(ORIGIN, None, None).unwrap();
    let mut config = GatewayConfig::new(ORIGIN, vec![3; 32]);
    config.rate_per_second = 10_000;
    config.rate_burst = 10_000;
    if public_pages {
        config.public_origin = Some("http://pages.localhost".into());
    }
    let gateway = match files {
        Some(files) => services
            .mount(GatewayBuilder::new(config, VERSION))
            .web_app(web::router(files))
            .build(),
        None => serve::app_gateway(&services, config),
    };
    gateway.unwrap().router()
}

fn app() -> Router {
    gateway(false, Some(FILES))
}

async fn send(app: &Router, request: Request<Body>) -> Reply {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        headers,
        body: body.to_vec(),
    }
}

fn get_on(host: &str, path: &str) -> Request<Body> {
    Request::get(path)
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap()
}

async fn get(app: &Router, path: &str) -> Reply {
    send(app, get_on("localhost", path)).await
}

#[tokio::test]
async fn app_routes_get_the_index() {
    let app = app();
    for path in [
        "/",
        "/p/0193a1b2",
        "/data/crm",
        "/sign-in",
        "/setup",
        "/search",
        "/admin",
    ] {
        let reply = get(&app, path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert!(reply.is_index(), "{path}");
        assert_eq!(
            reply.header(header::CONTENT_TYPE),
            "text/html; charset=utf-8"
        );
        assert_eq!(reply.header(header::CACHE_CONTROL), "no-store");
        let csp = reply.header(header::CONTENT_SECURITY_POLICY);
        for directive in [
            "script-src 'self'",
            "style-src 'self'",
            "font-src 'self'",
            "worker-src 'self'",
            "connect-src 'self' ws://localhost",
            "frame-ancestors 'none'",
        ] {
            assert!(csp.contains(directive), "{directive} in {csp}");
        }
        assert_eq!(reply.header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    }
}

#[tokio::test]
async fn hashed_assets_are_immutable_and_the_shell_files_revalidate() {
    let app = app();
    let js = get(&app, "/assets/index-3f2a9c.js").await;
    assert_eq!(js.status, StatusCode::OK);
    assert_eq!(js.body, b"console.log(1)");
    assert_eq!(
        js.header(header::CONTENT_TYPE),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        js.header(header::CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
    let font = get(&app, "/assets/inter-latin.woff2").await;
    assert_eq!(font.header(header::CONTENT_TYPE), "font/woff2");
    let worker = get(&app, "/sw.js").await;
    assert_eq!(worker.header(header::CACHE_CONTROL), "no-cache");
    let manifest = get(&app, "/manifest.webmanifest").await;
    assert_eq!(
        manifest.header(header::CONTENT_TYPE),
        "application/manifest+json"
    );
    assert_eq!(manifest.header(header::CACHE_CONTROL), "no-cache");
    let index = get(&app, "/index.html").await;
    assert!(index.is_index());
    assert_eq!(index.header(header::CACHE_CONTROL), "no-store");
}

#[tokio::test]
async fn missing_files_are_not_found() {
    let app = app();
    for path in [
        "/assets/index-gone.js",
        "/assets/",
        "/favicon.ico",
        "/p/x/a.png",
    ] {
        let reply = get(&app, path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!reply.is_index(), "{path}");
    }
    let post = send(
        &app,
        Request::post("/sign-in")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, ORIGIN)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(post.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn gateway_paths_never_get_the_index() {
    let app = app();
    let health = get(&app, "/healthz").await;
    assert_eq!(health.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&health.body).contains(VERSION));
    let status = get(&app, "/api/v1/auth/status").await;
    assert_eq!(status.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&status.body).contains("needsSetup"));
    for path in [
        "/api",
        "/api/v1/nothing-here",
        "/auth/sso/start",
        "/auth/sso/callback?code=x&state=y",
        "/auth/nothing-here",
        "/oauth/nothing-here",
        "/.well-known/nothing-here",
        "/collab",
        "/collab/x",
        "/readyz/x",
    ] {
        let reply = get(&app, path).await;
        assert!(!reply.is_index(), "{path}");
        assert_ne!(reply.status, StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn the_public_host_never_gets_the_app() {
    let app = gateway(true, Some(FILES));
    for path in ["/p/0193a1b2", "/s/AAAAAAAAAAAAAAAAAAAAAA", "/"] {
        let reply = send(&app, get_on("pages.localhost", path)).await;
        assert!(!reply.is_index(), "{path}");
        assert!(
            reply
                .header(header::CONTENT_SECURITY_POLICY)
                .contains("sandbox"),
            "{path}"
        );
    }
    // The same path on the app host is the app's page view.
    assert!(get(&app, "/p/0193a1b2").await.is_index());
}

#[tokio::test]
async fn the_embedded_app_is_mounted() {
    let app = gateway(false, None);
    let reply = get(&app, "/p/0193a1b2").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header(header::CACHE_CONTROL), "no-store");
    assert!(String::from_utf8_lossy(&reply.body).contains("<html"));
}

#[tokio::test]
async fn the_collab_path_is_kept_for_live_editing() {
    // Live editing answers at `/collab` (here without a workspace index,
    // so it is unavailable); the app's index never does.
    let path = "/collab/01933c6e-3c6e-7000-8000-000000000001/01933c6e-3c6e-7000-8000-000000000002";
    for app in [app(), gateway(false, None)] {
        let reply = get(&app, path).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!reply.is_index());
        assert_eq!(reply.header(header::CONTENT_TYPE), "application/json");
    }
}
