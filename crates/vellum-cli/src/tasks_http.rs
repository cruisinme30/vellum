// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The calendar feed, CalDAV and capture routes on the gateway.
//!
//! These routes are mounted cross-site: calendar apps, the mail relay and
//! Slack cannot send a CSRF token. So a browser session cookie never
//! counts here. The feed and capture routes carry their own secret or
//! signature. CalDAV needs an API token, sent either as a bearer token or
//! as the password of HTTP Basic sign-in (the user name is ignored),
//! because that is what most CalDAV apps can send.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use base64::Engine as _;
use vellum_api::auth::Authenticated;
use vellum_contracts::Principal;
use vellum_identity::Identity;
use vellum_tasks::raw::RawRequest;
use vellum_tasks::service::TasksService;

/// The largest request body accepted (an e-mail with small attachments).
pub const MAX_BODY: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct Routes {
    tasks: Arc<TasksService>,
    identity: Arc<Identity>,
}

/// The routers to mount cross-site, by prefix.
pub fn routers(tasks: &Arc<TasksService>, identity: &Arc<Identity>) -> Vec<(&'static str, Router)> {
    let state = Routes {
        tasks: Arc::clone(tasks),
        identity: Arc::clone(identity),
    };
    ["/feeds", "/dav", "/capture", "/connect"]
        .into_iter()
        .map(|prefix| {
            let router = Router::new()
                .route("/", any(handle))
                .route("/{*rest}", any(handle))
                .with_state(state.clone());
            (prefix, router)
        })
        .collect()
}

/// The API token a CalDAV app sends in the password slot of an HTTP Basic
/// `Authorization` header. It is looked up as a bearer token, never treated
/// as an account password.
fn basic_auth_token(request: &Request) -> Option<String> {
    let value = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let encoded = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (_, token) = text.split_once(':')?;
    Some(token.to_owned())
}

/// Who is calling, under the rules in the module docs.
fn caller(routes: &Routes, request: &Request, path: &str) -> Principal {
    let cookie = request
        .extensions()
        .get::<Authenticated>()
        .is_some_and(|a| a.csrf_binding.is_some());
    let principal = request
        .extensions()
        .get::<Principal>()
        .cloned()
        .filter(|_| !cookie)
        .unwrap_or_else(Principal::anonymous);
    if principal.user_id().is_some() || !path.starts_with("/dav") {
        return principal;
    }
    basic_auth_token(request)
        .and_then(|token| routes.identity.bearer(&token).ok())
        .map_or(principal, |resolved| resolved.principal)
}

async fn handle(
    State(routes): State<Routes>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    let path = uri.path().to_owned();
    let principal = caller(&routes, &request, &path);
    let method = request.method().as_str().to_ascii_uppercase();
    let headers = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect();
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_BODY).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response();
    };
    let raw = RawRequest {
        method,
        path,
        query: uri.query().unwrap_or_default().to_owned(),
        headers,
        body: body.to_vec(),
        principal,
    };
    let out = routes.tasks.handle_raw(raw).await;
    let mut response = Response::new(Body::from(out.body));
    *response.status_mut() =
        StatusCode::from_u16(out.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    for (name, value) in out.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::from_str(&value),
        ) {
            let _ = response.headers_mut().append(name, value);
        }
    }
    response
}
