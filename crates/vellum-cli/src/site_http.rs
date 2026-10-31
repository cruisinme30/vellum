// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Markdown for models and crawlers in `vellum serve` (R-AI-6, D295):
//! `/llms.txt`, `/sitemap.md`, and `?format=md` on every page URL.
//!
//! The pages come from [`vellum_mcp::site`] over the same workspace `/mcp`
//! uses, so what a visitor sees is exactly what they may read: the
//! signed-in user (session cookie or bearer token) sees their pages, and
//! an anonymous visitor or a share-link holder sees none.

use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_mcp::auth::{Principal, ScopeSet};
use vellum_mcp::ports::{Actor, Workspace};
use vellum_mcp::site;

/// The site's view of a gateway caller: a signed-in user, or nobody.
#[must_use]
pub fn actor(authenticated: &Authenticated) -> Actor {
    let principal = &authenticated.principal;
    let Some(user) = principal.user_id() else {
        return Actor::Anonymous;
    };
    let scopes: Vec<&str> = principal
        .scopes()
        .iter()
        .map(vellum_contracts::Scope::as_str)
        .collect();
    Actor::User(Principal {
        token_id: "session".to_owned(),
        user_id: user.to_string(),
        user_name: String::new(),
        client_id: "vellum-web".to_owned(),
        client_name: "Vellum".to_owned(),
        scopes: ScopeSet::parse(&scopes.join(" ")),
    })
}

/// Puts the gateway's caller in the request as the site's [`Actor`].
async fn with_actor(mut request: Request, next: Next) -> Response {
    let actor = request
        .extensions()
        .get::<Authenticated>()
        .map_or(Actor::Anonymous, actor);
    let _ = request.extensions_mut().insert(actor);
    next.run(request).await
}

/// `/llms.txt` and `/sitemap.md`, each to be mounted at its own path on
/// the gateway, which authenticates the caller first.
pub fn mounts(ws: &Arc<dyn Workspace>) -> [(&'static str, Router); 2] {
    [
        (
            "/llms.txt",
            site::llms_router(Arc::clone(ws)).layer(from_fn(with_actor)),
        ),
        (
            "/sitemap.md",
            site::sitemap_router(Arc::clone(ws)).layer(from_fn(with_actor)),
        ),
    ]
}

fn wants_markdown(request: &Request) -> bool {
    request
        .uri()
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == "format=md"))
}

/// Authenticates a `?format=md` request, which the gateway does not do
/// for the web app's paths. A missing or bad credential is anonymous: the
/// page then falls through to the app, which asks the visitor to sign in.
async fn authenticate(
    State(authenticator): State<Arc<dyn Authenticator>>,
    mut request: Request,
    next: Next,
) -> Response {
    if wants_markdown(&request) {
        let meta = RequestMeta {
            headers: request.headers().clone(),
            client_ip: request
                .extensions()
                .get::<ConnectInfo<std::net::SocketAddr>>()
                .map(|c| c.0.ip()),
            path: request.uri().path().to_owned(),
        };
        let who = authenticator
            .authenticate(&meta)
            .await
            .map_or(Actor::Anonymous, |a| actor(&a));
        let _ = request.extensions_mut().insert(who);
    }
    next.run(request).await
}

/// The web app, answering `?format=md` on a page URL with the page's
/// Markdown when the caller may read it.
pub fn markdown_pages(
    web_app: Router,
    ws: Arc<dyn Workspace>,
    authenticator: Arc<dyn Authenticator>,
) -> Router {
    web_app
        .layer(from_fn_with_state(ws, site::format_md))
        .layer(from_fn_with_state(authenticator, authenticate))
}
