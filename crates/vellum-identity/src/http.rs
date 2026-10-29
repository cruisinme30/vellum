// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The identity endpoints that set cookies or redirect.
//!
//! They cannot be typed operations. The binary mounts them on the gateway,
//! which applies its middleware and puts the caller's [`Principal`] in
//! each request:
//!
//! - [`browser_router`] at `/auth` (CSRF-checked): sign-in, sign-out,
//!   re-authentication, password change, break-glass and `OAuth` consent.
//! - [`oauth_router`] at `/oauth` (cross-site): authorize, token, revoke.
//! - [`well_known_router`] at `/.well-known` (cross-site): server metadata.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, Form, OriginalUri, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;
use vellum_contracts::{ApiError, Principal};
use vellum_kernel::{Error, ErrorKind, RequestId};

use crate::identity::{Identity, NewSession};
use crate::oauth::{Authorize, AuthorizeRequest, TokenRequest};

type Shared = State<Arc<Identity>>;

/// Where the web app shows its sign-in page; `next` is appended.
pub const SIGN_IN_PATH: &str = "/sign-in";

fn error_response(error: &Error, id: Option<Extension<RequestId>>) -> Response {
    let id = id.map_or_else(RequestId::generate, |Extension(id)| id);
    let status = StatusCode::from_u16(ApiError::status(error.kind()))
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, Json(ApiError::from_error(error, id))).into_response();
    if error.kind() == ErrorKind::RateLimited
        && let Some(secs) = error
            .message()
            .split_whitespace()
            .find_map(|w| w.parse::<u64>().ok())
        && let Ok(value) = HeaderValue::from_str(&secs.to_string())
    {
        let _ = response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

fn no_store(mut response: Response) -> Response {
    let _ = response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn client_ip(info: Option<&Extension<ConnectInfo<SocketAddr>>>) -> Option<IpAddr> {
    info.map(|Extension(ConnectInfo(addr))| addr.ip())
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

/// Reads a cookie by name.
#[must_use]
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn session_cookie(identity: &Identity, value: &str, max_age: Duration) -> HeaderValue {
    let config = identity.config();
    let secure = if config.https { "; Secure" } else { "" };
    let text = format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        config.cookie_name(),
        max_age.as_secs()
    );
    HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn signed_in(identity: &Identity, session: &NewSession) -> Response {
    let max_age = Duration::from_millis(
        u64::try_from(session.session.expires_at.unix_ms() - session.session.created_at.unix_ms())
            .unwrap_or(0),
    );
    let user = identity.user(session.session.user).ok();
    let mut response =
        Json(json!({ "user": user, "sessionId": session.session.id })).into_response();
    let _ = response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(identity, &session.secret, max_age),
    );
    no_store(response)
}

#[derive(Debug, Deserialize)]
struct SignIn {
    email: String,
    password: String,
}

async fn sign_in(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<SignIn>,
) -> Response {
    let ip = client_ip(info.as_ref());
    let worker = Arc::clone(&identity);
    let agent = user_agent(&headers).map(str::to_owned);
    // Argon2 is deliberately slow: keep it off the async workers.
    let result = tokio::task::spawn_blocking(move || {
        worker.sign_in(&input.email, &input.password, ip, agent.as_deref())
    })
    .await
    .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())));
    match result {
        Ok(session) => signed_in(&identity, &session),
        Err(e) => error_response(&e, id),
    }
}

async fn sign_out(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    headers: HeaderMap,
) -> Response {
    if let Some(value) = cookie(&headers, identity.config().cookie_name())
        && let Err(e) = identity.sign_out(value)
    {
        return error_response(&e, id);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    let _ = response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&identity, "", Duration::ZERO),
    );
    let _ = response.headers_mut().insert(
        "clear-site-data",
        HeaderValue::from_static("\"cache\", \"storage\""),
    );
    response
}

#[derive(Debug, Deserialize)]
struct Password {
    password: String,
}

async fn reauthenticate(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<Password>,
) -> Response {
    let Some(value) = cookie(&headers, identity.config().cookie_name()).map(str::to_owned) else {
        return error_response(
            &Error::new(ErrorKind::Unauthenticated, "sign in to continue"),
            id,
        );
    };
    let ip = client_ip(info.as_ref());
    let worker = Arc::clone(&identity);
    let result =
        tokio::task::spawn_blocking(move || worker.reauthenticate(&value, &input.password, ip))
            .await
            .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())));
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangePassword {
    current_password: Option<String>,
    new_password: String,
}

async fn change_password(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<ChangePassword>,
) -> Response {
    let user = match principal.require_user() {
        Ok(u) => u,
        Err(e) => return error_response(&e, id),
    };
    let keep = cookie(&headers, identity.config().cookie_name()).map(str::to_owned);
    let worker = Arc::clone(&identity);
    let result = tokio::task::spawn_blocking(move || {
        worker.change_password(
            user,
            keep.as_deref(),
            input.current_password.as_deref(),
            &input.new_password,
        )
    })
    .await
    .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())));
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
struct BreakGlass {
    code: String,
}

async fn break_glass(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<BreakGlass>,
) -> Response {
    match identity.redeem_break_glass(&input.code, client_ip(info.as_ref()), user_agent(&headers)) {
        Ok(session) => signed_in(&identity, &session),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Decision {
    consent_id: String,
    approve: bool,
}

async fn consent(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<Decision>,
) -> Response {
    let result = principal
        .require_user()
        .and_then(|user| identity.decide(user, &input.consent_id, input.approve));
    match result {
        Ok(url) => no_store(Json(json!({ "redirectTo": url })).into_response()),
        Err(e) => error_response(&e, id),
    }
}

/// The browser endpoints, to mount at `/auth`.
pub fn browser_router(identity: Arc<Identity>) -> Router {
    Router::new()
        .route("/sign-in", post(sign_in))
        .route("/sign-out", post(sign_out))
        .route("/reauthenticate", post(reauthenticate))
        .route("/password", post(change_password))
        .route("/break-glass", post(break_glass))
        .route("/consent", post(consent))
        .with_state(identity)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>{}</title></head>\
         <body><main>{body}</main></body></html>",
        escape(title)
    );
    let mut response = (status, html).into_response();
    let _ = response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    no_store(response)
}

const fn scope_description(scope: vellum_contracts::Scope) -> &'static str {
    use vellum_contracts::Scope;
    match scope {
        Scope::Read => "Read and search everything you can see",
        Scope::WriteDraft => "Propose changes to pages and rows for review",
        Scope::WriteDirect => "Change pages and rows directly",
        Scope::Work => "File and update issues, comments and tasks",
        Scope::Admin => "Administer the workspace",
    }
}

async fn authorize(
    State(identity): Shared,
    principal: Option<Extension<Principal>>,
    Query(request): Query<AuthorizeRequest>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let user = principal.and_then(|Extension(p)| p.user_id());
    let Some(user) = user else {
        let next = uri
            .path_and_query()
            .map_or("/oauth/authorize", axum::http::uri::PathAndQuery::as_str);
        let target = format!(
            "{SIGN_IN_PATH}?{}",
            serde_urlencoded::to_string([("next", next)]).unwrap_or_default()
        );
        return Redirect::to(&target).into_response();
    };
    match identity.authorize(user, &request).await {
        Authorize::Redirect(url) => no_store(Redirect::to(&url).into_response()),
        Authorize::Fail(message) => page(
            StatusCode::BAD_REQUEST,
            "Cannot connect this app",
            &format!(
                "<h1>Cannot connect this app</h1><p>{}</p>",
                escape(&message)
            ),
        ),
        Authorize::Consent(view) => {
            let scopes = view.scopes.iter().fold(String::new(), |mut out, s| {
                out.push_str("<li>");
                out.push_str(&escape(scope_description(*s)));
                out.push_str("</li>");
                out
            });
            page(
                StatusCode::OK,
                "Connect an app",
                &format!(
                    "<h1>Allow {name} to use your Vellum account?</h1>\
                     <p><strong>{name}</strong> ({client}) will be able to:</p><ul>{scopes}</ul>\
                     <p>You will be sent back to <strong>{host}</strong>. You can disconnect it \
                     at any time in your settings.</p>\
                     <form id=\"consent\" data-consent=\"{consent}\">\
                     <button type=\"submit\" name=\"decision\" value=\"approve\">Allow</button> \
                     <button type=\"submit\" name=\"decision\" value=\"deny\">Cancel</button></form>\
                     <p id=\"error\" role=\"alert\"></p>\
                     <script src=\"/oauth/consent.js\"></script>",
                    name = escape(&view.client_name),
                    client = escape(&view.client_id),
                    host = escape(&view.redirect_host),
                    consent = escape(&view.consent_id),
                ),
            )
        }
    }
}

const CONSENT_JS: &str = r"'use strict';
const form = document.getElementById('consent');
form.addEventListener('submit', async (event) => {
  event.preventDefault();
  const approve = event.submitter !== null && event.submitter.value === 'approve';
  const error = document.getElementById('error');
  try {
    const csrf = await (await fetch('/api/csrf', { credentials: 'same-origin' })).json();
    const response = await fetch('/auth/consent', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json', [csrf.header]: csrf.token },
      body: JSON.stringify({ consentId: form.dataset.consent, approve }),
    });
    const body = await response.json();
    if (response.ok) {
      window.location.assign(body.redirectTo);
    } else {
      error.textContent = body.message;
    }
  } catch {
    error.textContent = 'Something went wrong. Reload the page and try again.';
  }
});
";

async fn consent_js() -> Response {
    let mut response = CONSENT_JS.into_response();
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    let _ = headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

fn oauth_json<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    let mut response = no_store((status, Json(body)).into_response());
    let _ = response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

async fn token(State(identity): Shared, Form(request): Form<TokenRequest>) -> Response {
    match identity.token(&request) {
        Ok(tokens) => oauth_json(StatusCode::OK, &tokens),
        Err(e) => oauth_json(StatusCode::BAD_REQUEST, &e),
    }
}

#[derive(Debug, Deserialize)]
struct Revoke {
    token: String,
}

async fn revoke(State(identity): Shared, Form(request): Form<Revoke>) -> Response {
    match identity.revoke(&request.token) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => oauth_json(
            StatusCode::SERVICE_UNAVAILABLE,
            &json!({ "error": "temporarily_unavailable", "error_description": e.message() }),
        ),
    }
}

async fn userinfo(State(identity): Shared, headers: HeaderMap) -> Response {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    match identity.userinfo(token.trim()) {
        Ok(info) => oauth_json(StatusCode::OK, &info),
        Err(e) => {
            let mut response = oauth_json(
                StatusCode::UNAUTHORIZED,
                &json!({ "error": "invalid_token", "error_description": e.message() }),
            );
            let _ = response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer error=\"invalid_token\""),
            );
            response
        }
    }
}

/// The `OAuth` endpoints, to mount cross-site at `/oauth`.
pub fn oauth_router(identity: Arc<Identity>) -> Router {
    Router::new()
        .route("/authorize", get(authorize))
        .route("/consent.js", get(consent_js))
        .route("/token", post(token))
        .route("/revoke", post(revoke))
        .route("/userinfo", get(userinfo))
        .with_state(identity)
}

async fn metadata(State(identity): Shared) -> Response {
    let mut response = Json(identity.oauth_metadata()).into_response();
    let _ = response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    let _ = response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// The discovery documents, to mount cross-site at `/.well-known`.
pub fn well_known_router(identity: Arc<Identity>) -> Router {
    Router::new()
        .route("/oauth-authorization-server", get(metadata))
        .with_state(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_are_parsed() {
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; vellum_session=vss_x; b=2"),
        );
        assert_eq!(cookie(&headers, "vellum_session"), Some("vss_x"));
        assert_eq!(cookie(&headers, "missing"), None);
        assert_eq!(escape("<b>\"x\"</b>"), "&lt;b&gt;&quot;x&quot;&lt;/b&gt;");
    }
}
