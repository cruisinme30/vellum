// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The identity endpoints that set cookies or redirect.
//!
//! They cannot be typed operations. The binary mounts them on the gateway,
//! which applies its middleware and puts the caller's [`Principal`] in
//! each request:
//!
//! - [`browser_router`] at `/auth` (CSRF-checked): sign-in and its second
//!   step, sign-out, re-authentication, password change, break-glass,
//!   `OAuth` consent, and the single sign-on start and callback
//!   (top-level `GET`s).
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
use crate::mfa::{Proof, SignIn};
use crate::oauth::{Authorize, AuthorizeRequest, TokenRequest};
use crate::webauthn::{Assertion, Registration};

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
struct Credentials {
    email: String,
    password: String,
}

async fn sign_in(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> Response {
    let ip = client_ip(info.as_ref());
    let worker = Arc::clone(&identity);
    let agent = user_agent(&headers).map(str::to_owned);
    // Argon2 is deliberately slow: keep it off the async workers.
    let result = tokio::task::spawn_blocking(move || {
        worker.sign_in_password(&input.email, &input.password, ip, agent.as_deref())
    })
    .await
    .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())));
    match result {
        Ok(SignIn::Session(session)) => signed_in(&identity, &session),
        // The password was right; the browser answers the second step.
        Ok(SignIn::Verify(ticket)) => {
            no_store(Json(json!({ "secondFactor": ticket })).into_response())
        }
        Ok(SignIn::Enrol(ticket)) => no_store(Json(json!({ "enrol": ticket })).into_response()),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Verify {
    ticket: String,
    code: Option<String>,
    passkey: Option<Assertion>,
}

/// The second step of a sign-in: a code or a passkey for the ticket.
async fn sign_in_verify(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(input): Json<Verify>,
) -> Response {
    let proof = Proof {
        code: input.code,
        passkey: input.passkey,
    };
    match identity.verify_sign_in(&input.ticket, &proof, client_ip(info.as_ref())) {
        Ok(session) => signed_in(&identity, &session),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
struct TicketOnly {
    ticket: String,
}

/// The passkey ceremony for a ticket: an assertion to sign in, or a new
/// passkey to enrol.
async fn sign_in_passkey_options(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    Json(input): Json<TicketOnly>,
) -> Response {
    match identity.ticket_passkey_options(&input.ticket) {
        Ok(options) => no_store(Json(json!({ "options": options })).into_response()),
        Err(e) => error_response(&e, id),
    }
}

/// An authenticator app to set up with an enrolment ticket.
async fn sign_in_authenticator(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    Json(input): Json<TicketOnly>,
) -> Response {
    match identity.ticket_totp_setup(&input.ticket) {
        Ok(setup) => no_store(Json(setup).into_response()),
        Err(e) => error_response(&e, id),
    }
}

#[derive(Debug, Deserialize)]
struct NewPasskey {
    name: String,
    credential: Registration,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrol {
    ticket: String,
    code: Option<String>,
    passkey: Option<NewPasskey>,
}

/// Adds the first factor with an enrolment ticket and signs in; the
/// answer carries the recovery codes, shown once.
async fn sign_in_enrol(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(input): Json<Enrol>,
) -> Response {
    let passkey = input
        .passkey
        .as_ref()
        .map(|p| (p.name.as_str(), &p.credential));
    match identity.enrol_with_ticket(
        &input.ticket,
        input.code.as_deref(),
        passkey,
        client_ip(info.as_ref()),
    ) {
        Ok((session, codes)) => {
            let mut response = signed_in(&identity, &session);
            let user = identity.user(session.session.user).ok();
            *response.body_mut() = axum::body::Body::from(
                json!({ "user": user, "sessionId": session.session.id, "recoveryCodes": codes })
                    .to_string(),
            );
            response
        }
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
#[serde(deny_unknown_fields)]
struct Reauthenticate {
    password: String,
    code: Option<String>,
    passkey: Option<Assertion>,
    ticket: Option<String>,
}

async fn reauthenticate(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<Reauthenticate>,
) -> Response {
    let Some(value) = cookie(&headers, identity.config().cookie_name()).map(str::to_owned) else {
        return error_response(
            &Error::new(ErrorKind::Unauthenticated, "sign in to continue"),
            id,
        );
    };
    let ip = client_ip(info.as_ref());
    let worker = Arc::clone(&identity);
    let result = tokio::task::spawn_blocking(move || {
        let proof = Proof {
            code: input.code,
            passkey: input.passkey,
        };
        worker.reauthenticate(&value, &input.password, &proof, input.ticket.as_deref(), ip)
    })
    .await
    .unwrap_or_else(|e| Err(Error::new(ErrorKind::Internal, e.to_string())));
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(&e, id),
    }
}

/// A passkey ceremony to confirm the signed-in person.
async fn reauth_passkey_options(
    State(identity): Shared,
    id: Option<Extension<RequestId>>,
    headers: HeaderMap,
) -> Response {
    let Some(value) = cookie(&headers, identity.config().cookie_name()) else {
        return error_response(
            &Error::new(ErrorKind::Unauthenticated, "sign in to continue"),
            id,
        );
    };
    match identity.reauth_passkey_options(value) {
        Ok(ceremony) => no_store(Json(ceremony).into_response()),
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

/// Where the provider sends the browser back, under the `/auth` mount.
pub const SSO_CALLBACK_PATH: &str = "/auth/sso/callback";

fn sso_state_cookie(identity: &Identity, value: &str, max_age: Duration) -> HeaderValue {
    let config = identity.config();
    let (name, secure) = if config.https {
        ("__Host-vellum_sso", "; Secure")
    } else {
        ("vellum_sso", "")
    };
    let text = format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        max_age.as_secs()
    );
    HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""))
}

const fn sso_state_name(identity: &Identity) -> &'static str {
    if identity.config().https {
        "__Host-vellum_sso"
    } else {
        "vellum_sso"
    }
}

fn sso_failed(message: &str) -> Response {
    page(
        StatusCode::FORBIDDEN,
        "Single sign-on failed",
        &format!(
            "<h1>Single sign-on failed</h1><p>{}</p><p><a href=\"{SIGN_IN_PATH}\">Back to sign-in</a></p>",
            escape(message)
        ),
    )
}

#[derive(Debug, Deserialize)]
struct SsoStartQuery {
    next: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    reauth: Option<String>,
}

/// Sends the browser to the provider's sign-in page.
async fn sso_start(State(identity): Shared, Query(query): Query<SsoStartQuery>) -> Response {
    let (provider, sso) = match identity.sso_provider(query.provider.as_deref()) {
        Ok(found) => found,
        Err(e) => return sso_failed(e.message()),
    };
    let redirect_uri = format!("{}{SSO_CALLBACK_PATH}", identity.config().base());
    let reauth = query
        .reauth
        .as_deref()
        .is_some_and(|v| matches!(v, "1" | "true"));
    match sso
        .start(&redirect_uri, query.next.as_deref(), reauth)
        .await
    {
        Ok(started) => {
            let mut response = Redirect::to(&started.url).into_response();
            let _ = response.headers_mut().insert(
                header::SET_COOKIE,
                sso_state_cookie(
                    &identity,
                    &format!("{provider}.{}", started.state),
                    crate::sso::PENDING_LIFETIME,
                ),
            );
            no_store(response)
        }
        Err(e) => sso_failed(e.message()),
    }
}

#[derive(Debug, Deserialize)]
struct SsoCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Where the provider returns: verifies the sign-in, opens a session and
/// goes on to the page the user started from.
async fn sso_callback(
    State(identity): Shared,
    headers: HeaderMap,
    Query(query): Query<SsoCallbackQuery>,
) -> Response {
    // The state cookie names the provider the sign-in started with.
    let pending = cookie(&headers, sso_state_name(&identity));
    let (provider, cookie_state) = match pending.and_then(|p| p.split_once('.')) {
        Some((provider, state)) => (Some(provider), Some(state)),
        None => (None, None),
    };
    let sso = match identity.sso_provider(provider) {
        Ok((_, sso)) => sso,
        Err(e) => {
            let mut failed = sso_failed(e.message());
            let _ = failed.headers_mut().append(
                header::SET_COOKIE,
                sso_state_cookie(&identity, "", Duration::ZERO),
            );
            return no_store(failed);
        }
    };
    let mut response = match (&query.error, &query.code, &query.state) {
        (Some(error), _, _) => {
            let detail = query.error_description.as_deref().unwrap_or(error);
            sso_failed(&format!("The sign-in provider said: {detail}"))
        }
        (None, Some(code), Some(state)) => {
            let redirect_uri = format!("{}{SSO_CALLBACK_PATH}", identity.config().base());
            match sso.finish(&redirect_uri, state, code, cookie_state).await {
                Ok(done) => {
                    match identity.sso_sign_in_with(&sso, &done.identity, user_agent(&headers)) {
                        Ok(session) => {
                            // A sign-in in this browser replaces its old session.
                            if let Some(old) = cookie(&headers, identity.config().cookie_name()) {
                                let _ = identity.sign_out(old);
                            }
                            let max_age = Duration::from_millis(
                                u64::try_from(
                                    session.session.expires_at.unix_ms()
                                        - session.session.created_at.unix_ms(),
                                )
                                .unwrap_or(0),
                            );
                            let mut ok = Redirect::to(&done.next).into_response();
                            let _ = ok.headers_mut().append(
                                header::SET_COOKIE,
                                session_cookie(&identity, &session.secret, max_age),
                            );
                            ok
                        }
                        Err(e) => sso_failed(e.message()),
                    }
                }
                Err(e) => {
                    tracing::info!(event = "identity.sso_failed", error = %e, "single sign-on failed");
                    sso_failed(e.message())
                }
            }
        }
        _ => sso_failed("The sign-in provider sent an incomplete answer."),
    };
    let _ = response.headers_mut().append(
        header::SET_COOKIE,
        sso_state_cookie(&identity, "", Duration::ZERO),
    );
    no_store(response)
}

/// The browser endpoints, to mount at `/auth`.
pub fn browser_router(identity: Arc<Identity>) -> Router {
    Router::new()
        .route("/sso/start", get(sso_start))
        .route("/sso/callback", get(sso_callback))
        .route("/sign-in", post(sign_in))
        .route("/sign-in/verify", post(sign_in_verify))
        .route("/sign-in/passkey-options", post(sign_in_passkey_options))
        .route("/sign-in/authenticator", post(sign_in_authenticator))
        .route("/sign-in/enrol", post(sign_in_enrol))
        .route(
            "/reauthenticate/passkey-options",
            post(reauth_passkey_options),
        )
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
