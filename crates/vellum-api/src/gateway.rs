// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Assembling the gateway: routes, middleware and probes.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, MatchedPath, RawPathParams, RawQuery, Request, State};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get, on, post};
use axum::{Extension, Json};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::Instrument as _;
use vellum_contracts::{HttpMethod, Readiness, RequestContext, Service};
use vellum_kernel::{Error, ErrorKind, RequestId};

use crate::auth::{AnonymousOnly, Authenticated, Authenticator, RequestMeta};
use crate::breaker::BreakerPolicy;
use crate::config::{GatewayConfig, host_of};
use crate::error::ErrorResponse;
use crate::idempotency::{
    Begin, IDEMPOTENCY_KEY, IDEMPOTENT_REPLAYED, IdempotencyStore, MAX_KEY_LEN,
    MemoryIdempotencyStore, ScopedKey, StoredResponse,
};
use crate::public::{NothingPublished, PublicSite};
use crate::registry::{RegisteredOp, Registry};
use crate::request_id::REQUEST_ID;
use crate::security::{CSRF_HEADER, Surface, apply_headers, check_csrf, csrf_token};
use crate::timing::{RequestObservation, collect};
use crate::{graphql, input, openapi};
use vellum_security::rate::{Quota, RateLimiter};

/// Largest response body stored for idempotent replay.
const MAX_STORED_RESPONSE: usize = 1024 * 1024;
/// How long one service's readiness probe may take.
const READINESS_TIMEOUT: Duration = Duration::from_secs(2);
/// Callers tracked by the rate limiter before drained ones are evicted.
const RATE_LIMIT_KEYS: usize = 100_000;

/// A router mounted by the binary (collab rooms, MCP, downloads, auth
/// callbacks).
struct Mount {
    prefix: String,
    router: Router,
    csrf_exempt: bool,
}

/// Builds a [`Gateway`].
pub struct GatewayBuilder {
    config: GatewayConfig,
    version: &'static str,
    services: Vec<Arc<dyn Service>>,
    authenticator: Arc<dyn Authenticator>,
    idempotency: Arc<dyn IdempotencyStore>,
    public_site: Arc<dyn PublicSite>,
    breaker: BreakerPolicy,
    mounts: Vec<Mount>,
}

impl std::fmt::Debug for GatewayBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayBuilder")
            .field("config", &self.config)
            .field(
                "services",
                &self.services.iter().map(|s| s.name()).collect::<Vec<_>>(),
            )
            .field(
                "mounts",
                &self.mounts.iter().map(|m| &m.prefix).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl GatewayBuilder {
    /// A gateway with no services, anonymous callers only and in-memory
    /// idempotency.
    #[must_use]
    pub fn new(config: GatewayConfig, version: &'static str) -> Self {
        Self {
            config,
            version,
            services: Vec::new(),
            authenticator: Arc::new(AnonymousOnly),
            idempotency: Arc::new(MemoryIdempotencyStore::default()),
            public_site: Arc::new(NothingPublished),
            breaker: BreakerPolicy::default(),
            mounts: Vec::new(),
        }
    }

    /// Serves a service's operations.
    #[must_use]
    pub fn service(mut self, service: Arc<dyn Service>) -> Self {
        self.services.push(service);
        self
    }

    /// Sets the authenticator (the identity service).
    #[must_use]
    pub fn authenticator(mut self, authenticator: Arc<dyn Authenticator>) -> Self {
        self.authenticator = authenticator;
        self
    }

    /// Sets the idempotency store.
    #[must_use]
    pub fn idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency = store;
        self
    }

    /// Sets what the public-pages origin serves.
    #[must_use]
    pub fn public_site(mut self, site: Arc<dyn PublicSite>) -> Self {
        self.public_site = site;
        self
    }

    /// Sets the circuit-breaker policy for every service.
    #[must_use]
    pub const fn breaker_policy(mut self, policy: BreakerPolicy) -> Self {
        self.breaker = policy;
        self
    }

    /// Mounts a raw router under `prefix` (`/collab`, `/mcp`). It gets the
    /// same edge middleware, authentication, rate limiting and CSRF checks;
    /// the [`Authenticated`] caller and [`RequestId`] are in the request
    /// extensions.
    #[must_use]
    pub fn mount(mut self, prefix: impl Into<String>, router: Router) -> Self {
        self.mounts.push(Mount {
            prefix: prefix.into(),
            router,
            csrf_exempt: false,
        });
        self
    }

    /// Mounts a router whose unsafe requests legitimately come from other
    /// sites (the `SAML` assertion consumer, the `OAuth` token endpoint), so
    /// the CSRF check is skipped. Such routes must carry their own
    /// protection (signed assertions, PKCE).
    #[must_use]
    pub fn mount_cross_site(mut self, prefix: impl Into<String>, router: Router) -> Self {
        self.mounts.push(Mount {
            prefix: prefix.into(),
            router,
            csrf_exempt: true,
        });
        self
    }

    /// Validates every operation and assembles the routes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] for an invalid or duplicate operation, or a
    /// `GraphQL` schema that cannot be built.
    pub fn build(self) -> Result<Gateway, Error> {
        let registry = Registry::new(self.services, self.breaker)?;
        let openapi = openapi::document(&registry, self.version);
        let graphql = graphql::schema(&registry, self.version)
            .map_err(|e| Error::new(ErrorKind::Internal, format!("invalid GraphQL schema: {e}")))?;
        let quota = Quota::new(
            self.config.rate_burst.max(1),
            Duration::from_secs(1) * self.config.rate_burst.max(1)
                / self.config.rate_per_second.max(1),
        )
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "the rate limit must be positive"))?;
        let limiter = RateLimiter::new(quota, RATE_LIMIT_KEYS);
        let state = Arc::new(Shared {
            config: self.config,
            version: self.version,
            registry,
            authenticator: self.authenticator,
            idempotency: self.idempotency,
            limiter,
            draining: AtomicBool::new(false),
            openapi,
            graphql,
        });
        let app = app_router(&state, self.mounts);
        let public = crate::public::router(self.public_site);
        let public = edge(public, &state, Surface::Public);
        Ok(Gateway { state, app, public })
    }
}

/// State shared by every request.
struct Shared {
    config: GatewayConfig,
    version: &'static str,
    registry: Registry,
    authenticator: Arc<dyn Authenticator>,
    idempotency: Arc<dyn IdempotencyStore>,
    limiter: RateLimiter<String>,
    draining: AtomicBool,
    openapi: Value,
    graphql: async_graphql::dynamic::Schema,
}

/// The assembled gateway.
#[derive(Clone)]
pub struct Gateway {
    state: Arc<Shared>,
    app: Router,
    public: Router,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("registry", &self.state.registry)
            .finish_non_exhaustive()
    }
}

impl Gateway {
    /// The router to serve. Requests for the public-pages host go to the
    /// public origin; everything else to the app.
    pub fn router(&self) -> Router {
        let app = self.app.clone();
        let public = self.public.clone();
        let public_host = self.state.config.public_host();
        Router::new().fallback_service(tower::service_fn(move |request: Request| {
            let is_public = public_host
                .as_deref()
                .is_some_and(|host| request_host(request.headers()) == host);
            let router = if is_public {
                public.clone()
            } else {
                app.clone()
            };
            tower::ServiceExt::oneshot(router, request)
        }))
    }

    /// Marks the process as draining: `/readyz` fails so load balancers stop
    /// sending traffic, while in-flight and new requests still complete.
    pub fn start_draining(&self) {
        self.state.draining.store(true, Ordering::SeqCst);
        tracing::info!("draining: readiness now fails");
    }

    /// The OpenAPI 3.1 document.
    #[must_use]
    pub fn openapi(&self) -> &Value {
        &self.state.openapi
    }

    /// The `GraphQL` schema in SDL.
    #[must_use]
    pub fn graphql_sdl(&self) -> String {
        self.state.graphql.sdl()
    }

    /// How long shutdown waits for in-flight requests.
    #[must_use]
    pub fn drain_timeout(&self) -> Duration {
        self.state.config.drain_timeout
    }
}

fn request_host(headers: &HeaderMap) -> String {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| host_of(h.trim()))
        .unwrap_or_default()
}

fn client_ip(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

fn request_id(request: &Request) -> RequestId {
    request
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_else(RequestId::generate)
}

/// The outermost layer: request ID, span, `Server-Timing`, security
/// headers, panics, compression.
fn edge(router: Router, state: &Arc<Shared>, surface: Surface) -> Router {
    router
        .layer(CatchPanicLayer::custom(
            |_: Box<dyn std::any::Any + Send>| {
                ErrorResponse::of(ErrorKind::Internal, "panic", RequestId::generate())
                    .into_response()
            },
        ))
        .layer(from_fn_with_state((Arc::clone(state), surface), observe))
        .layer(CompressionLayer::new())
}

async fn observe(
    State((state, surface)): State<(Arc<Shared>, Surface)>,
    mut request: Request,
    next: Next,
) -> Response {
    let id = crate::request_id::from_headers(request.headers());
    let _ = request.extensions_mut().insert(id);
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());
    let obs = RequestObservation::begin(request.method().as_str(), &route, &id.to_string());
    let span = obs.span().clone();
    let (mut response, timings) = collect(next.run(request).instrument(span)).await;
    let done = obs.finish(response.status().as_u16(), &timings);
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&id.to_string()) {
        let _ = headers.insert(REQUEST_ID, value);
    }
    if let Ok(value) = HeaderValue::from_str(&done.server_timing) {
        let _ = headers.insert("server-timing", value);
    }
    apply_headers(headers, surface, &state.config);
    response
}

#[derive(Clone, Copy)]
struct Protect {
    csrf: bool,
}

async fn protect(
    State((state, policy)): State<(Arc<Shared>, Protect)>,
    mut request: Request,
    next: Next,
) -> Response {
    let id = request_id(&request);
    let ip = client_ip(&request);
    let meta = RequestMeta {
        headers: request.headers().clone(),
        client_ip: ip,
        path: request.uri().path().to_owned(),
    };
    let authenticated = match state.authenticator.authenticate(&meta).await {
        Ok(a) => a,
        Err(e) => return ErrorResponse::new(e, id).into_response(),
    };
    let caller = authenticated
        .principal
        .user_id()
        .map(|u| format!("user:{u}"))
        .or_else(|| ip.map(|ip| format!("ip:{ip}")))
        .unwrap_or_else(|| "anonymous".to_owned());
    if let Err(limited) = state.limiter.check(&caller, Instant::now()) {
        let wait = limited.retry_after;
        let mut response =
            ErrorResponse::of(ErrorKind::RateLimited, "too many requests; slow down", id)
                .into_response();
        let seconds = wait.as_secs().max(1).to_string();
        if let Ok(value) = HeaderValue::from_str(&seconds) {
            let _ = response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }
    let csrf = if policy.csrf {
        check_csrf(
            request.method(),
            request.headers(),
            authenticated.csrf_binding.as_ref(),
            &state.config,
        )
    } else {
        Ok(())
    };
    if let Err(e) = csrf {
        return ErrorResponse::new(e, id).into_response();
    }
    let _ = request
        .extensions_mut()
        .insert(authenticated.principal.clone());
    let _ = request.extensions_mut().insert(authenticated);
    if request.headers().contains_key(&IDEMPOTENCY_KEY)
        && crate::security::is_unsafe(request.method())
    {
        return idempotent(&state, caller, request, next).await;
    }
    next.run(request).await
}

async fn idempotent(state: &Shared, caller: String, request: Request, next: Next) -> Response {
    let id = request_id(&request);
    let key = request
        .headers()
        .get(&IDEMPOTENCY_KEY)
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= MAX_KEY_LEN)
        .map(str::to_owned);
    let Some(key) = key else {
        return ErrorResponse::of(
            ErrorKind::InvalidInput,
            "Idempotency-Key must be 1 to 255 visible characters",
            id,
        )
        .into_response();
    };
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, state.config.max_body_bytes).await else {
        return ErrorResponse::of(ErrorKind::InvalidInput, "the request body is too large", id)
            .into_response();
    };
    let mut hasher = Sha256::new();
    hasher.update(parts.method.as_str());
    hasher.update([0]);
    hasher.update(parts.uri.to_string());
    hasher.update([0]);
    hasher.update(&bytes);
    let fingerprint: [u8; 32] = hasher.finalize().into();
    let scoped = ScopedKey { caller, key };
    match state
        .idempotency
        .begin(scoped.clone(), fingerprint, state.config.idempotency_ttl)
        .await
    {
        Begin::Replay(stored) => {
            let mut response = (stored.status, stored.body).into_response();
            if let Some(ct) = stored
                .content_type
                .and_then(|ct| HeaderValue::from_str(&ct).ok())
            {
                let _ = response.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            let _ = response
                .headers_mut()
                .insert(IDEMPOTENT_REPLAYED, HeaderValue::from_static("true"));
            response
        }
        Begin::InFlight => ErrorResponse::of(
            ErrorKind::Conflict,
            "a request with this Idempotency-Key is still running",
            id,
        )
        .into_response(),
        Begin::Mismatch => {
            let mut response = ErrorResponse::of(
                ErrorKind::InvalidInput,
                "this Idempotency-Key was used for a different request",
                id,
            )
            .into_response();
            *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
            response
        }
        Begin::Proceed => {
            let response = next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
            if response.status().is_server_error() {
                state.idempotency.abandon(scoped).await;
                return response;
            }
            let (parts, body) = response.into_parts();
            if let Ok(bytes) = to_bytes(body, MAX_STORED_RESPONSE).await {
                {
                    let stored = StoredResponse {
                        status: parts.status,
                        content_type: parts
                            .headers
                            .get(header::CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                        body: bytes.to_vec(),
                    };
                    state.idempotency.complete(scoped, stored).await;
                    Response::from_parts(parts, Body::from(bytes))
                }
            } else {
                state.idempotency.abandon(scoped).await;
                ErrorResponse::of(ErrorKind::Internal, "response too large to store", id)
                    .into_response()
            }
        }
    }
}

fn layered(router: Router, state: &Arc<Shared>, csrf: bool) -> Router {
    router
        .layer(from_fn_with_state(
            (Arc::clone(state), Protect { csrf }),
            protect,
        ))
        .layer(RequestBodyLimitLayer::new(state.config.max_body_bytes))
}

fn app_router(state: &Arc<Shared>, mounts: Vec<Mount>) -> Router {
    let mut api = Router::new()
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/graphql", post(graphql_post))
        .route("/api/graphql/schema.graphql", get(graphql_sdl))
        .route("/api/csrf", get(csrf));
    for registered in state.registry.operations() {
        let path = format!("/api{}", registered.op.path());
        api = api.route(&path, operation_route(registered));
    }
    let api = layered(
        api.fallback(api_not_found).with_state(Arc::clone(state)),
        state,
        true,
    );
    let mut app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(Arc::clone(state))
        .merge(api);
    for mount in mounts {
        app = app.nest(
            &mount.prefix,
            layered(mount.router, state, !mount.csrf_exempt),
        );
    }
    edge(app, state, Surface::App)
}

fn operation_route(registered: &Arc<RegisteredOp>) -> MethodRouter<Arc<Shared>> {
    let filter = match registered.op.method() {
        HttpMethod::Get => axum::routing::MethodFilter::GET,
        HttpMethod::Post => axum::routing::MethodFilter::POST,
        HttpMethod::Put => axum::routing::MethodFilter::PUT,
        HttpMethod::Patch => axum::routing::MethodFilter::PATCH,
        HttpMethod::Delete => axum::routing::MethodFilter::DELETE,
    };
    let registered = Arc::clone(registered);
    on(
        filter,
        move |Extension(id): Extension<RequestId>,
              Extension(authenticated): Extension<Authenticated>,
              params: RawPathParams,
              RawQuery(query): RawQuery,
              headers: HeaderMap,
              body: Bytes| {
            let registered = Arc::clone(&registered);
            async move {
                rest(
                    &registered,
                    id,
                    authenticated,
                    &params,
                    query.as_deref(),
                    &headers,
                    &body,
                )
                .await
            }
        },
    )
}

async fn rest(
    registered: &RegisteredOp,
    id: RequestId,
    authenticated: Authenticated,
    params: &RawPathParams,
    query: Option<&str>,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let op = &registered.op;
    let pairs: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let body = op.method().has_body().then_some(body);
    let input = match input::build(op, &pairs, query, body) {
        Ok(input) => input,
        Err(e) => return ErrorResponse::new(e, id).into_response(),
    };
    let ctx = RequestContext {
        request_id: id,
        principal: authenticated.principal,
        deadline: Instant::now() + op.time_budget(),
        traceparent: headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    };
    match registered.invoke(ctx, input).await {
        Ok(output) => Json(output).into_response(),
        Err(e) => ErrorResponse::new(e, id).into_response(),
    }
}

async fn api_not_found(Extension(id): Extension<RequestId>) -> Response {
    ErrorResponse::of(ErrorKind::NotFound, "no such API route", id).into_response()
}

async fn openapi_json(State(state): State<Arc<Shared>>) -> Response {
    Json(state.openapi.clone()).into_response()
}

async fn graphql_sdl(State(state): State<Arc<Shared>>) -> Response {
    let mut response = state.graphql.sdl().into_response();
    let _ = response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

async fn graphql_post(
    State(state): State<Arc<Shared>>,
    Extension(id): Extension<RequestId>,
    Extension(authenticated): Extension<Authenticated>,
    headers: HeaderMap,
    Json(request): Json<async_graphql::Request>,
) -> Response {
    let data = graphql::GraphqlRequest {
        request_id: id,
        authenticated,
        traceparent: headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    };
    let response = state.graphql.execute(request.data(data)).await;
    Json(response).into_response()
}

async fn csrf(
    State(state): State<Arc<Shared>>,
    Extension(id): Extension<RequestId>,
    Extension(auth): Extension<Authenticated>,
) -> Response {
    auth.csrf_binding.map_or_else(
        || ErrorResponse::of(ErrorKind::Unauthenticated, "no browser session", id).into_response(),
        |binding| {
            let mut response = Json(json!({ "token": csrf_token(&state.config.csrf_key, &binding), "header": CSRF_HEADER.as_str() })).into_response();
            let _ = response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        },
    )
}

async fn healthz(State(state): State<Arc<Shared>>) -> Response {
    Json(json!({ "status": "ok", "version": state.version })).into_response()
}

async fn readyz(State(state): State<Arc<Shared>>) -> Response {
    if state.draining.load(Ordering::SeqCst) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "draining" })),
        )
            .into_response();
    }
    let mut ready = true;
    let mut services = serde_json::Map::new();
    for service in state.registry.services() {
        let readiness = tokio::time::timeout(READINESS_TIMEOUT, service.readiness())
            .await
            .unwrap_or_else(|_| Readiness::NotReady("readiness probe timed out".into()));
        ready &= !matches!(readiness, Readiness::NotReady(_));
        let _ = services.insert(
            service.name().to_owned(),
            serde_json::to_value(&readiness).unwrap_or(Value::Null),
        );
    }
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({ "status": if ready { "ready" } else { "not_ready" }, "services": services });
    (status, Json(body)).into_response()
}
