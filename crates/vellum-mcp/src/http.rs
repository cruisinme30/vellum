// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Streamable HTTP transport at `/mcp`.
//!
//! Every JSON-RPC message is its own POST. Requests get a JSON response,
//! except `subscriptions/listen`, which gets a long-lived SSE stream. There
//! are no sessions, no GET stream and no resumption: GET and DELETE answer
//! `405`. The endpoint is an `OAuth` 2.1 resource server; see [`crate::auth`].

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use serde_json::{Value, json};

use crate::auth::{self, Principal, ResourceConfig, TokenError, TokenVerifier};
use crate::protocol::{
    Incoming, LEGACY_PROTOCOL_VERSION, META_PROTOCOL_VERSION, Outgoing, PROTOCOL_VERSION, RpcError,
    SUPPORTED_VERSIONS,
};
use crate::server::{Listener, McpServer, Reply, RequestCtx};
use crate::setup;

/// Settings of the HTTP transport.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Public URLs and the token issuer.
    pub resource: ResourceConfig,
    /// Extra origins allowed to call `/mcp` from a browser, besides the
    /// deployment's own origin.
    pub allowed_origins: Vec<String>,
    /// How often an idle listen stream sends a keep-alive comment.
    pub keep_alive: Duration,
}

impl HttpConfig {
    /// Defaults for a deployment at `base_url` whose identity service is
    /// also the token issuer.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_owned();
        Self {
            resource: ResourceConfig {
                base_url: base.clone(),
                issuer: base,
            },
            allowed_origins: Vec::new(),
            keep_alive: Duration::from_secs(15),
        }
    }
}

/// Shared state of the HTTP transport.
pub struct McpHttp {
    /// The protocol core.
    pub server: Arc<McpServer>,
    /// The token port.
    pub verifier: Arc<dyn TokenVerifier>,
    /// Settings.
    pub config: HttpConfig,
}

impl std::fmt::Debug for McpHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHttp")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl McpHttp {
    /// Authenticates a bearer token for this endpoint.
    ///
    /// # Errors
    ///
    /// `None` when no token was sent; otherwise the token error.
    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<Principal, Option<TokenError>> {
        let value = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(None)?;
        let (scheme, token) = value.split_once(' ').ok_or(Some(TokenError::Invalid))?;
        if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() {
            return Err(Some(TokenError::Invalid));
        }
        let verified = self.verifier.verify(token.trim()).await.map_err(Some)?;
        auth::admit(
            verified,
            &self.config.resource.resource(),
            SystemTime::now(),
        )
        .map_err(Some)
    }

    fn origin_allowed(&self, origin: &str) -> bool {
        let own = origin_of(&self.config.resource.base_url);
        origin.eq_ignore_ascii_case(&own)
            || self
                .config
                .allowed_origins
                .iter()
                .any(|o| o.eq_ignore_ascii_case(origin))
    }
}

fn origin_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let host = rest.split('/').next().unwrap_or_default();
    format!("{scheme}://{host}")
}

/// The routes: `/mcp`, the Protected Resource Metadata documents, and the
/// setup endpoints.
pub fn router(state: Arc<McpHttp>) -> Router {
    Router::new()
        .nest("/mcp", endpoint_router(Arc::clone(&state)))
        .nest("/.well-known", metadata_router(state))
}

/// The endpoint and its setup routes, relative to `/mcp`: for a server
/// that mounts them itself.
pub fn endpoint_router(state: Arc<McpHttp>) -> Router {
    Router::new()
        .route(
            "/",
            post(handle_post)
                .get(method_not_allowed)
                .delete(method_not_allowed),
        )
        .route("/setup", get(setup::snippets_handler))
        .route("/test-connection", post(setup::test_connection_handler))
        .with_state(state)
}

/// The Protected Resource Metadata documents, relative to `/.well-known`.
pub fn metadata_router(state: Arc<McpHttp>) -> Router {
    Router::new()
        .route("/oauth-protected-resource", get(resource_metadata))
        .route("/oauth-protected-resource/mcp", get(resource_metadata))
        .with_state(state)
}

async fn method_not_allowed() -> Response {
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    let _ = r
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

async fn resource_metadata(State(state): State<Arc<McpHttp>>) -> Response {
    let mut r = axum::Json(state.config.resource.metadata()).into_response();
    let _ = r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    r
}

/// A request id: the gateway's `X-Request-Id` when present, otherwise a
/// new one.
#[must_use]
pub fn request_id(headers: &HeaderMap) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(id) = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic()))
    {
        return id.to_owned();
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "req_{nanos:x}{:04x}",
        COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    let mut r = (status, axum::Json(body.clone())).into_response();
    let _ = r
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn error_response(
    status: StatusCode,
    id: Option<crate::protocol::RequestId>,
    error: RpcError,
) -> Response {
    json_response(status, &Outgoing::Error { id, error }.to_value())
}

/// Decodes a header that may use the `=?base64?…?=` sentinel.
fn decode_header(value: &str) -> Option<String> {
    value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
        .map_or_else(
            || Some(value.to_owned()),
            |b64| {
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
            },
        )
}

/// Checks the mirrored headers against the body.
fn validate_headers(
    headers: &HeaderMap,
    msg: &Incoming,
) -> Result<Option<String>, (StatusCode, RpcError)> {
    let text = |name: &str| -> Result<Option<String>, (StatusCode, RpcError)> {
        headers
            .get(name)
            .map(|v| {
                v.to_str().map(str::to_owned).map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        RpcError::header_mismatch(format!("{name} has invalid characters")),
                    )
                })
            })
            .transpose()
    };
    let version = text("mcp-protocol-version")?;
    let body_version = msg
        .meta()
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
        .and_then(Value::as_str);
    if msg.method == "initialize" && version.is_none() {
        // A legacy handshake carries no version header.
        return Ok(None);
    }
    let Some(version) = version else {
        return Err((
            StatusCode::BAD_REQUEST,
            RpcError::header_mismatch("MCP-Protocol-Version header is missing"),
        ));
    };
    if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            RpcError::unsupported_version(&version),
        ));
    }
    if let Some(bv) = body_version
        && bv != version
    {
        return Err((
            StatusCode::BAD_REQUEST,
            RpcError::header_mismatch(format!(
                "MCP-Protocol-Version header value '{version}' does not match body value '{bv}'"
            )),
        ));
    }
    if version == LEGACY_PROTOCOL_VERSION || msg.id.is_none() {
        return Ok(Some(version));
    }
    match text("mcp-method")? {
        Some(m) if m == msg.method => {}
        Some(m) => {
            return Err((
                StatusCode::BAD_REQUEST,
                RpcError::header_mismatch(format!(
                    "Mcp-Method header value '{m}' does not match body value '{}'",
                    msg.method
                )),
            ));
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                RpcError::header_mismatch("Mcp-Method header is missing"),
            ));
        }
    }
    let name_field = match msg.method.as_str() {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(field) = name_field {
        let body_value = msg.str_param(field).unwrap_or_default();
        match text("mcp-name")?.as_deref().map(decode_header) {
            Some(Some(h)) if h == body_value => {}
            Some(Some(h)) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    RpcError::header_mismatch(format!(
                        "Mcp-Name header value '{h}' does not match body value '{body_value}'"
                    )),
                ));
            }
            Some(None) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    RpcError::header_mismatch("Mcp-Name header is not valid base64"),
                ));
            }
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    RpcError::header_mismatch("Mcp-Name header is missing"),
                ));
            }
        }
    }
    Ok(Some(version))
}

fn accepts(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|a| {
            a.contains("application/json") || a.contains("text/event-stream") || a.contains("*/*")
        })
}

fn with_common(mut r: Response, request_id: &str, started: Instant) -> Response {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(request_id) {
        let _ = h.insert("x-request-id", v);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    if let Ok(v) = HeaderValue::from_str(&format!("mcp;dur={ms:.1}")) {
        let _ = h.insert("server-timing", v);
    }
    r
}

async fn handle_post(
    State(state): State<Arc<McpHttp>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let request_id = request_id(&headers);
    let r = post_inner(&state, &headers, &body, &request_id).await;
    with_common(r, &request_id, started)
}

async fn post_inner(
    state: &Arc<McpHttp>,
    headers: &HeaderMap,
    body: &[u8],
    request_id: &str,
) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && !state.origin_allowed(origin)
    {
        return error_response(
            StatusCode::FORBIDDEN,
            None,
            RpcError::invalid_request("origin not allowed"),
        );
    }
    let principal = match state.authenticate(headers).await {
        Ok(p) => p,
        Err(e) => {
            let mut r = error_response(
                StatusCode::UNAUTHORIZED,
                None,
                RpcError::new(
                    crate::protocol::code::UNAUTHENTICATED,
                    e.as_ref()
                        .map_or_else(|| "authentication required".to_owned(), ToString::to_string),
                ),
            );
            if let Ok(v) =
                HeaderValue::from_str(&state.config.resource.unauthorized_challenge(e.as_ref()))
            {
                let _ = r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
            }
            return r;
        }
    };
    if !accepts(headers) {
        return error_response(
            StatusCode::NOT_ACCEPTABLE,
            None,
            RpcError::invalid_request("Accept must allow application/json and text/event-stream"),
        );
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            None,
            RpcError::new(crate::protocol::code::PARSE_ERROR, "body is not UTF-8"),
        );
    };
    let msg = match Incoming::parse(text) {
        Ok(m) => m,
        Err((id, e)) => return error_response(StatusCode::BAD_REQUEST, id, e),
    };
    let version = match validate_headers(headers, &msg) {
        Ok(v) => v,
        Err((status, e)) => return error_response(status, msg.id.clone(), e),
    };
    let ctx = RequestCtx {
        principal,
        request_id: request_id.to_owned(),
        fallback_version: version,
    };
    match state.server.handle(msg, &ctx).await {
        Reply::Result { id, result } => {
            json_response(StatusCode::OK, &Outgoing::Result { id, result }.to_value())
        }
        Reply::Accepted => StatusCode::ACCEPTED.into_response(),
        Reply::Error {
            id,
            error,
            status,
            challenge,
            retry_after,
        } => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
            let mut r = error_response(code, id, error);
            if let Some(scopes) = challenge
                && let Ok(v) = HeaderValue::from_str(
                    &state.config.resource.insufficient_scope_challenge(&scopes),
                )
            {
                let _ = r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
            }
            if let Some(wait) = retry_after {
                let secs = wait.as_secs().max(1);
                if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                    let _ = r.headers_mut().insert(header::RETRY_AFTER, v);
                }
            }
            r
        }
        Reply::Listen(listener) => sse(listener, Arc::clone(state), headers.clone()),
    }
}

fn event(msg: &Outgoing) -> Bytes {
    Bytes::from(format!("event: message\ndata: {}\n\n", msg.to_line()))
}

enum Phase {
    Ack(Listener),
    Open(Listener),
    Done,
}

/// The listen stream. Its token is checked again at every keep-alive and
/// before each notification, and the stream closes once the token no
/// longer verifies (revoked, expired, account blocked): RT-047.
fn sse(listener: Listener, state: Arc<McpHttp>, headers: HeaderMap) -> Response {
    let keep_alive = state.config.keep_alive;
    let still_valid = move || {
        let state = Arc::clone(&state);
        let headers = headers.clone();
        async move { state.authenticate(&headers).await.is_ok() }
    };
    let stream = futures_util::stream::unfold(Phase::Ack(listener), move |phase| {
        let still_valid = still_valid.clone();
        async move {
            match phase {
                Phase::Ack(l) => Some((
                    Ok::<_, Infallible>(event(&l.acknowledgement())),
                    Phase::Open(l),
                )),
                Phase::Open(mut l) => {
                    let next = tokio::select! {
                        next = l.next() => Some(next),
                        () = tokio::time::sleep(keep_alive) => None,
                    };
                    if !still_valid().await {
                        return Some((Ok(event(&l.closing())), Phase::Done));
                    }
                    match next {
                        Some(Some(n)) => Some((Ok(event(&n)), Phase::Open(l))),
                        Some(None) => Some((Ok(event(&l.closing())), Phase::Done)),
                        None => Some((Ok(Bytes::from_static(b":\n\n")), Phase::Open(l))),
                    }
                }
                Phase::Done => None,
            }
        }
    });
    let mut r = Response::new(Body::from_stream(stream));
    let h = r.headers_mut();
    let _ = h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    let _ = h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let _ = h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

/// The versions header a client should send, for snippets and tests.
#[must_use]
pub const fn protocol_version() -> &'static str {
    PROTOCOL_VERSION
}

/// A JSON body for an MCP request, for tests and the connection check.
#[must_use]
pub fn request_body(id: i64, method: &str, mut params: Value) -> Value {
    if let Some(p) = params.as_object_mut() {
        let _ = p.insert(
            "_meta".into(),
            json!({
                META_PROTOCOL_VERSION: PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": { "name": "vellum-test-connection", "version": env!("CARGO_PKG_VERSION") }
            }),
        );
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_sentinels() {
        assert_eq!(
            decode_header("=?base64?SGVsbG8sIOS4lueVjA==?=").as_deref(),
            Some("Hello, 世界")
        );
        assert_eq!(decode_header("search").as_deref(), Some("search"));
        assert_eq!(decode_header("=?base64?!!?="), None);
    }

    #[test]
    fn origins_compare_scheme_and_host() {
        assert_eq!(
            origin_of("https://vellum.northstar.dev/x"),
            "https://vellum.northstar.dev"
        );
    }

    #[test]
    fn request_ids_are_reused_or_minted() {
        let mut h = HeaderMap::new();
        assert!(request_id(&h).starts_with("req_"));
        let _ = h.insert("x-request-id", HeaderValue::from_static("abc"));
        assert_eq!(request_id(&h), "abc");
    }
}
