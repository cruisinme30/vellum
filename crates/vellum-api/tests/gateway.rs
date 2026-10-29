// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Contract and behaviour tests for the gateway, over a sample service.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::routing::post;
use http::{Request, StatusCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_api::breaker::BreakerPolicy;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_api::security::csrf_token;
use vellum_contracts::{
    BoxFuture, CredentialKind, HttpMethod, Operation, Principal, Readiness, Scope, ScopeSet,
    Service,
};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const APP: &str = "https://vellum.northstar.dev";
const KEY: [u8; 32] = [9; 32];
const BINDING: [u8; 32] = [5; 32];

/// A note, the sample service's object.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Note {
    /// The note's ID.
    id: u32,
    /// The text.
    text: String,
    /// Labels.
    labels: Vec<String>,
    /// How urgent.
    priority: Priority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Priority {
    Low,
    High,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetNote {
    space: String,
    id: u32,
    shout: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[expect(dead_code, reason = "the space is only routed")]
struct CreateNote {
    space: String,
    text: String,
    #[serde(default)]
    labels: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Empty {}

#[derive(Debug, Serialize, JsonSchema)]
struct Count {
    created: u64,
}

struct Notes {
    created: AtomicUsize,
    ready: Readiness,
}

impl Service for Notes {
    fn name(&self) -> &'static str {
        "notes"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let me = Arc::clone(&self);
        vec![
            Operation::query(
                "notes.get",
                "/v1/spaces/{space}/notes/{id}",
                Scope::Read,
                |ctx, input: GetNote| async move {
                    let _ = ctx.principal.require_user()?;
                    if input.id == 404 {
                        return Err(Error::new(ErrorKind::NotFound, "no such note"));
                    }
                    let text = format!("note {} in {}", input.id, input.space);
                    let text = if input.shout == Some(true) {
                        text.to_uppercase()
                    } else {
                        text
                    };
                    Ok(Note {
                        id: input.id,
                        text,
                        labels: vec!["a".into()],
                        priority: Priority::High,
                    })
                },
            )
            .summary("Gets a note"),
            Operation::mutation(
                "notes.create",
                HttpMethod::Post,
                "/v1/spaces/{space}/notes",
                Scope::WriteDirect,
                move |_ctx, input: CreateNote| {
                    let me = Arc::clone(&me);
                    async move {
                        let n = me.created.fetch_add(1, Ordering::SeqCst) + 1;
                        Ok(Note {
                            id: u32::try_from(n).unwrap_or(0),
                            text: input.text,
                            labels: input.labels,
                            priority: Priority::Low,
                        })
                    }
                },
            ),
            Operation::query("notes.count", "/v1/notes/count", Scope::Read, {
                let me = Arc::clone(&self);
                move |_ctx, _input: Empty| {
                    let me = Arc::clone(&me);
                    async move {
                        Ok(Count {
                            created: me.created.load(Ordering::SeqCst) as u64,
                        })
                    }
                }
            })
            .allow_anonymous(),
            Operation::mutation(
                "notes.purge",
                HttpMethod::Delete,
                "/v1/notes",
                Scope::Admin,
                |_ctx, _input: Empty| async { Ok(Count { created: 0 }) },
            )
            .fresh_auth(Duration::from_mins(5)),
            Operation::query(
                "notes.slow",
                "/v1/notes/slow",
                Scope::Read,
                |_ctx, _input: Empty| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok(Count { created: 0 })
                },
            )
            .budget(Duration::from_millis(30)),
            Operation::query(
                "notes.broken",
                "/v1/notes/broken",
                Scope::Read,
                |_ctx, _input: Empty| async {
                    Err::<Count, _>(Error::new(ErrorKind::Unavailable, "disk on fire"))
                },
            ),
        ]
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        let ready = self.ready.clone();
        Box::pin(async move { ready })
    }
}

/// Bearer `full`, `read`, `stale` and the `session` cookie.
struct FakeAuth;

impl Authenticator for FakeAuth {
    fn authenticate<'a>(
        &'a self,
        request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>> {
        let user = UserId::from_u128(42);
        let header = |n: &str| {
            request
                .headers
                .get(n)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        let result = match header("authorization").as_str() {
            "Bearer full" => Ok(Authenticated {
                principal: Principal::user(
                    user,
                    CredentialKind::ApiToken,
                    ScopeSet::FULL,
                    Timestamp::now(),
                ),
                csrf_binding: None,
            }),
            "Bearer read" => Ok(Authenticated {
                principal: Principal::user(
                    user,
                    CredentialKind::ApiToken,
                    ScopeSet::from_scopes([Scope::Read]),
                    Timestamp::now(),
                ),
                csrf_binding: None,
            }),
            "Bearer stale" => Ok(Authenticated {
                principal: Principal::user(
                    user,
                    CredentialKind::ApiToken,
                    ScopeSet::FULL,
                    Timestamp::from_unix_ms(1).unwrap_or_else(|_| Timestamp::now()),
                ),
                csrf_binding: None,
            }),
            "" if header("cookie").contains("vellum_session=ok") => Ok(Authenticated {
                principal: Principal::user(
                    user,
                    CredentialKind::Session,
                    ScopeSet::FULL,
                    Timestamp::now(),
                ),
                csrf_binding: Some(BINDING),
            }),
            "" => Ok(Authenticated::anonymous()),
            _ => Err(Error::new(ErrorKind::Unauthenticated, "invalid token")),
        };
        Box::pin(async move { result })
    }
}

fn config() -> GatewayConfig {
    let mut c = GatewayConfig::new(APP, KEY.to_vec());
    c.public_origin = Some("https://pages.northstar.dev".into());
    c
}

fn gateway_with(config: GatewayConfig, ready: Readiness) -> Result<(Gateway, Arc<Notes>), Error> {
    let notes = Arc::new(Notes {
        created: AtomicUsize::new(0),
        ready,
    });
    let mounted = Router::new().route(
        "/echo",
        post(|Extension(p): Extension<Principal>| async move {
            p.user_id().map(|u| u.to_string()).unwrap_or_default()
        }),
    );
    let gateway = GatewayBuilder::new(config, "1.0.0-test")
        .service(Arc::clone(&notes) as Arc<dyn Service>)
        .authenticator(Arc::new(FakeAuth))
        .breaker_policy(BreakerPolicy {
            failure_threshold: 3,
            cool_down: Duration::from_mins(1),
        })
        .mount("/raw", mounted)
        .build()?;
    Ok((gateway, notes))
}

fn gateway() -> Result<(Gateway, Arc<Notes>), Error> {
    gateway_with(config(), Readiness::Ready)
}

struct Reply {
    status: StatusCode,
    headers: http::HeaderMap,
    body: Value,
    text: String,
}

async fn send(
    gateway: &Gateway,
    request: Request<Body>,
) -> Result<Reply, Box<dyn std::error::Error>> {
    let response = gateway.router().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply {
        status,
        headers,
        body,
        text,
    })
}

fn get(uri: &str, auth: &str) -> Result<Request<Body>, http::Error> {
    let mut b = Request::get(uri).header("host", "vellum.northstar.dev");
    if !auth.is_empty() {
        b = b.header("authorization", auth);
    }
    b.body(Body::empty())
}

fn post_json(uri: &str, auth: &str, body: &Value) -> Result<Request<Body>, http::Error> {
    let mut b = Request::post(uri)
        .header("host", "vellum.northstar.dev")
        .header("content-type", "application/json");
    if !auth.is_empty() {
        b = b.header("authorization", auth);
    }
    b.body(Body::from(body.to_string()))
}

#[tokio::test]
async fn rest_round_trip_matches_the_openapi_schema() -> TestResult {
    let (gw, _) = gateway()?;
    let reply = send(
        &gw,
        get("/api/v1/spaces/eng/notes/7?shout=true", "Bearer read")?,
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(
        reply.body,
        json!({"id": 7, "text": "NOTE 7 IN ENG", "labels": ["a"], "priority": "high"})
    );
    // The response validates against the schema the OpenAPI document promises.
    let doc = gw.openapi();
    let schema_ref = doc
        .pointer("/paths/~1v1~1spaces~1{space}~1notes~1{id}/get/responses/200/content/application~1json/schema/$ref")
        .and_then(Value::as_str)
        .ok_or("no response schema")?;
    let name = schema_ref.trim_start_matches("#/components/schemas/");
    let mut schema = doc
        .pointer(&format!("/components/schemas/{name}"))
        .cloned()
        .ok_or("missing component")?;
    if let Some(map) = schema.as_object_mut() {
        let _ = map.insert(
            "components".into(),
            doc.get("components").cloned().unwrap_or(Value::Null),
        );
    }
    let validator = jsonschema::validator_for(&schema)?;
    assert!(validator.is_valid(&reply.body));
    assert!(!validator.is_valid(&json!({"id": "x"})));
    Ok(())
}

#[tokio::test]
async fn openapi_document_is_valid_3_1() -> TestResult {
    let (gw, _) = gateway()?;
    let text = serde_json::to_string(gw.openapi())?;
    let spec = oas3::from_json(&text)?;
    assert_eq!(spec.openapi, "3.1.1");
    let ops: Vec<String> = spec
        .operations()
        .filter_map(|(_, _, op)| op.operation_id.clone())
        .collect();
    for id in ["notesGet", "notesCreate", "notesCount", "notesPurge"] {
        assert!(ops.iter().any(|o| o == id), "missing {id}");
    }
    let reply = send(&gw, get("/api/openapi.json", "")?).await?;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(&reply.body, gw.openapi());
    let create = gw
        .openapi()
        .pointer("/paths/~1v1~1spaces~1{space}~1notes/post")
        .ok_or("no create")?;
    assert_eq!(create.get("x-vellum-scope"), Some(&json!("write:direct")));
    assert!(create.to_string().contains("Idempotency-Key"));
    let purge = gw
        .openapi()
        .pointer("/paths/~1v1~1notes/delete")
        .ok_or("no purge")?;
    assert_eq!(purge.get("x-vellum-fresh-auth-seconds"), Some(&json!(300)));
    Ok(())
}

#[tokio::test]
async fn scopes_and_authentication() -> TestResult {
    let (gw, _) = gateway()?;
    let body = json!({"text": "hi"});
    let anon = send(&gw, post_json("/api/v1/spaces/eng/notes", "", &body)?).await?;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anon.body.get("code"), Some(&json!("unauthenticated")));
    assert!(
        anon.body
            .get("request_id")
            .and_then(Value::as_str)
            .is_some()
    );
    let read = send(
        &gw,
        post_json("/api/v1/spaces/eng/notes", "Bearer read", &body)?,
    )
    .await?;
    assert_eq!(read.status, StatusCode::FORBIDDEN);
    let full = send(
        &gw,
        post_json("/api/v1/spaces/eng/notes", "Bearer full", &body)?,
    )
    .await?;
    assert_eq!(full.status, StatusCode::OK, "{}", full.text);
    assert_eq!(full.body.get("text"), Some(&json!("hi")));
    let bad = send(&gw, get("/api/v1/notes/count", "Bearer forged")?).await?;
    assert_eq!(
        bad.status,
        StatusCode::UNAUTHORIZED,
        "a bad credential is never downgraded to anonymous"
    );
    let open = send(&gw, get("/api/v1/notes/count", "")?).await?;
    assert_eq!(open.status, StatusCode::OK, "anonymous-friendly operation");
    let invalid = send(&gw, get("/api/v1/spaces/eng/notes/x", "Bearer read")?).await?;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    let missing = send(&gw, get("/api/v1/spaces/eng/notes/404", "Bearer read")?).await?;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let nowhere = send(&gw, get("/api/v1/nowhere", "Bearer read")?).await?;
    assert_eq!(nowhere.status, StatusCode::NOT_FOUND);
    assert_eq!(nowhere.body.get("code"), Some(&json!("not_found")));
    Ok(())
}

#[tokio::test]
async fn admin_actions_need_a_recent_sign_in() -> TestResult {
    let (gw, _) = gateway()?;
    let req = |auth: &str| {
        Request::delete("/api/v1/notes")
            .header("host", "vellum.northstar.dev")
            .header("authorization", auth)
            .body(Body::empty())
    };
    let stale = send(&gw, req("Bearer stale")?).await?;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        stale.body.get("message"),
        Some(&json!("reauthentication required"))
    );
    let challenge = stale
        .headers
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(challenge.contains("insufficient_user_authentication"));
    assert_eq!(send(&gw, req("Bearer full")?).await?.status, StatusCode::OK);
    assert_eq!(
        send(&gw, req("Bearer read")?).await?.status,
        StatusCode::FORBIDDEN
    );
    Ok(())
}

#[tokio::test]
async fn csrf_protects_cookie_sessions() -> TestResult {
    let (gw, _) = gateway()?;
    let token = csrf_token(&KEY, &BINDING);
    let session_post = |csrf: Option<&str>, origin: &str| {
        let mut b = Request::post("/api/v1/spaces/eng/notes")
            .header("host", "vellum.northstar.dev")
            .header("cookie", "vellum_session=ok")
            .header("origin", origin)
            .header("content-type", "application/json");
        if let Some(t) = csrf {
            b = b.header("x-csrf-token", t);
        }
        b.body(Body::from(r#"{"text":"x"}"#))
    };
    assert_eq!(
        send(&gw, session_post(None, APP)?).await?.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&gw, session_post(Some("wrong"), APP)?).await?.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&gw, session_post(Some(&token), "https://evil.example")?)
            .await?
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&gw, session_post(Some(&token), APP)?).await?.status,
        StatusCode::OK
    );
    // The app fetches its token from /api/csrf.
    let fetched = send(
        &gw,
        Request::get("/api/csrf")
            .header("host", "vellum.northstar.dev")
            .header("cookie", "vellum_session=ok")
            .body(Body::empty())?,
    )
    .await?;
    assert_eq!(fetched.body.get("token"), Some(&json!(token)));
    assert_eq!(
        send(&gw, get("/api/csrf", "Bearer full")?).await?.status,
        StatusCode::UNAUTHORIZED
    );
    // Mounted routers are protected the same way and see the principal.
    let raw = |csrf: Option<&str>| {
        let mut b = Request::post("/raw/echo")
            .header("host", "vellum.northstar.dev")
            .header("cookie", "vellum_session=ok");
        if let Some(t) = csrf {
            b = b.header("x-csrf-token", t);
        }
        b.body(Body::empty())
    };
    assert_eq!(send(&gw, raw(None)?).await?.status, StatusCode::FORBIDDEN);
    let ok = send(&gw, raw(Some(&token))?).await?;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.text, UserId::from_u128(42).to_string());
    Ok(())
}

#[tokio::test]
async fn idempotent_writes_replay() -> TestResult {
    let (gw, notes) = gateway()?;
    let req = |key: &str, text: &str| {
        Request::post("/api/v1/spaces/eng/notes")
            .header("host", "vellum.northstar.dev")
            .header("authorization", "Bearer full")
            .header("idempotency-key", key)
            .header("content-type", "application/json")
            .body(Body::from(json!({"text": text}).to_string()))
    };
    let first = send(&gw, req("k1", "a")?).await?;
    let again = send(&gw, req("k1", "a")?).await?;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.body, again.body);
    assert_eq!(
        again
            .headers
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    assert_eq!(notes.created.load(Ordering::SeqCst), 1, "ran once");
    assert_eq!(
        send(&gw, req("k1", "b")?).await?.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(send(&gw, req("k2", "a")?).await?.status, StatusCode::OK);
    assert_eq!(notes.created.load(Ordering::SeqCst), 2);
    let long = "x".repeat(300);
    assert_eq!(
        send(&gw, req(&long, "a")?).await?.status,
        StatusCode::BAD_REQUEST
    );
    Ok(())
}

#[tokio::test]
async fn timeouts_and_circuit_breaker() -> TestResult {
    let (gw, _) = gateway()?;
    let slow = send(&gw, get("/api/v1/notes/slow", "Bearer read")?).await?;
    assert_eq!(slow.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(slow.body.get("retryable"), Some(&json!(true)));
    // The slow call counted as one failure; two more open the breaker.
    for _ in 0..2 {
        let r = send(&gw, get("/api/v1/notes/broken", "Bearer read")?).await?;
        assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.body.get("message"), Some(&json!("disk on fire")));
    }
    let fast_fail = send(&gw, get("/api/v1/spaces/eng/notes/1", "Bearer read")?).await?;
    assert_eq!(fast_fail.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        fast_fail.body.get("message"),
        Some(&json!("the notes service is temporarily unavailable"))
    );
    Ok(())
}

#[tokio::test]
async fn edge_headers() -> TestResult {
    let (gw, _) = gateway()?;
    let id = vellum_kernel::RequestId::generate().to_string();
    let request = Request::get("/api/v1/notes/count")
        .header("host", "vellum.northstar.dev")
        .header("x-request-id", &id)
        .body(Body::empty())?;
    let reply = send(&gw, request).await?;
    let h = |n: &str| {
        reply
            .headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(h("x-request-id"), id);
    assert!(h("server-timing").contains("total;dur="));
    assert!(h("server-timing").contains("traceparent;desc=\"00-"));
    assert!(h("content-security-policy").contains("default-src 'none'"));
    assert_eq!(h("x-content-type-options"), "nosniff");
    assert!(h("strict-transport-security").contains("max-age="));
    assert_eq!(h("cross-origin-opener-policy"), "same-origin");
    Ok(())
}

#[tokio::test]
async fn joins_the_callers_trace_and_counts_the_route() -> TestResult {
    let metrics = vellum_observe::RouteMetrics::new();
    let gw = GatewayBuilder::new(config(), "1.0.0-test")
        .service(Arc::new(Notes {
            created: AtomicUsize::new(0),
            ready: Readiness::Ready,
        }) as Arc<dyn Service>)
        .authenticator(Arc::new(FakeAuth))
        .route_metrics(metrics.clone())
        .build()?;
    let trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    let request = Request::get("/api/v1/notes/count")
        .header("host", "vellum.northstar.dev")
        .header("traceparent", format!("00-{trace}-00f067aa0ba902b7-01"))
        .body(Body::empty())?;
    let reply = send(&gw, request).await?;
    let timing = reply
        .headers
        .get("server-timing")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // The browser joins its own timings to this trace through the header.
    assert!(
        timing.contains(&format!("traceparent;desc=\"00-{trace}-")),
        "{timing}"
    );
    let routes = metrics.snapshot(1);
    let route = routes
        .iter()
        .find(|r| r.route == "/api/v1/notes/count")
        .ok_or("the route was not counted")?;
    assert_eq!(route.method, "GET");
    assert_eq!(route.counts.requests, 1);
    assert_eq!(metrics.in_flight(), 0);
    Ok(())
}

#[tokio::test]
async fn health_and_readiness() -> TestResult {
    let (gw, _) = gateway()?;
    let health = send(&gw, get("/healthz", "")?).await?;
    assert_eq!(health.status, StatusCode::OK);
    assert_eq!(health.body.get("version"), Some(&json!("1.0.0-test")));
    let ready = send(&gw, get("/readyz", "")?).await?;
    assert_eq!(ready.status, StatusCode::OK);
    assert_eq!(
        ready.body.pointer("/services/notes/status"),
        Some(&json!("ready"))
    );
    gw.start_draining();
    assert_eq!(
        send(&gw, get("/readyz", "")?).await?.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        send(&gw, get("/healthz", "")?).await?.status,
        StatusCode::OK,
        "still alive while draining"
    );

    let (down, _) = gateway_with(config(), Readiness::NotReady("index rebuilding".into()))?;
    let reply = send(&down, get("/readyz", "")?).await?;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        reply.body.pointer("/services/notes/reason"),
        Some(&json!("index rebuilding"))
    );
    let (degraded, _) = gateway_with(config(), Readiness::Degraded("blob store down".into()))?;
    assert_eq!(
        send(&degraded, get("/readyz", "")?).await?.status,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
async fn rate_limits_per_caller() -> TestResult {
    let mut c = config();
    c.rate_per_second = 1;
    c.rate_burst = 2;
    let (gw, _) = gateway_with(c, Readiness::Ready)?;
    let statuses = [
        send(&gw, get("/api/v1/notes/count", "Bearer read")?)
            .await?
            .status,
        send(&gw, get("/api/v1/notes/count", "Bearer read")?)
            .await?
            .status,
        send(&gw, get("/api/v1/notes/count", "Bearer read")?)
            .await?
            .status,
    ];
    assert_eq!(
        statuses,
        [
            StatusCode::OK,
            StatusCode::OK,
            StatusCode::TOO_MANY_REQUESTS
        ]
    );
    Ok(())
}

#[tokio::test]
async fn graphql_covers_every_operation() -> TestResult {
    let (gw, notes) = gateway()?;
    let sdl = gw.graphql_sdl();
    for needle in [
        "notesGet(input: GetNoteInput!): Note!",
        "notesCreate(input: CreateNoteInput!): Note!",
        "id: Int64!",
        "notesPurge: Count!",
        "enum Priority",
        "notesCount: Count!",
    ] {
        assert!(sdl.contains(needle), "SDL lacks `{needle}`:\n{sdl}");
    }
    let gql = |query: &str, auth: &str| post_json("/api/graphql", auth, &json!({ "query": query }));
    let read = send(
        &gw,
        gql(
            r#"{ notesGet(input: {space: "eng", id: 3}) { id text priority labels } }"#,
            "Bearer read",
        )?,
    )
    .await?;
    assert_eq!(
        read.body,
        json!({"data": {"notesGet": {"id": 3, "text": "note 3 in eng", "priority": "high", "labels": ["a"]}}}),
        "{}",
        read.text
    );
    let created = send(&gw, gql(r#"mutation { notesCreate(input: {space: "eng", text: "via graphql", labels: ["x"]}) { id text } }"#, "Bearer full")?).await?;
    assert_eq!(
        created.body.pointer("/data/notesCreate/text"),
        Some(&json!("via graphql")),
        "{}",
        created.text
    );
    assert_eq!(notes.created.load(Ordering::SeqCst), 1);
    let denied = send(
        &gw,
        gql(
            r#"mutation { notesCreate(input: {space: "eng", text: "no"}) { id } }"#,
            "Bearer read",
        )?,
    )
    .await?;
    assert_eq!(
        denied.body.pointer("/errors/0/extensions/code"),
        Some(&json!("permission_denied")),
        "{}",
        denied.text
    );
    assert!(
        denied
            .body
            .pointer("/errors/0/extensions/request_id")
            .is_some()
    );
    let count = send(&gw, gql("{ notesCount { created } apiVersion }", "")?).await?;
    assert_eq!(
        count.body,
        json!({"data": {"notesCount": {"created": 1}, "apiVersion": "1.0.0-test"}})
    );
    let sdl_reply = send(&gw, get("/api/graphql/schema.graphql", "")?).await?;
    assert!(sdl_reply.text.contains("type Mutation"));
    Ok(())
}

#[tokio::test]
async fn public_origin_is_separate() -> TestResult {
    let (gw, _) = gateway()?;
    let token = "A".repeat(22);
    let request = Request::get(format!("/s/{token}"))
        .header("host", "pages.northstar.dev")
        .body(Body::empty())?;
    let reply = send(&gw, request).await?;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let h = |n: &str| {
        reply
            .headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    assert!(h("content-type").starts_with("text/html"));
    assert!(h("content-security-policy").contains("sandbox"));
    assert_eq!(h("referrer-policy"), "no-referrer");
    assert!(!reply.text.contains("<script"));
    // The API is not reachable on the public origin, and shares not on the app origin.
    let api = Request::get("/api/v1/notes/count")
        .header("host", "pages.northstar.dev")
        .body(Body::empty())?;
    assert!(send(&gw, api).await?.text.contains("page not found"));
    let share_on_app = Request::get(format!("/s/{token}"))
        .header("host", "vellum.northstar.dev")
        .body(Body::empty())?;
    assert_eq!(send(&gw, share_on_app).await?.status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn the_web_app_gets_only_paths_the_gateway_does_not_own() -> TestResult {
    let shell = Router::new().fallback(|| async { "app shell" });
    let gw = GatewayBuilder::new(config(), "1.0.0-test")
        .service(Arc::new(Notes {
            created: AtomicUsize::new(0),
            ready: Readiness::Ready,
        }))
        .authenticator(Arc::new(FakeAuth))
        .mount(
            "/raw",
            Router::new().route("/echo", post(|| async { "echo" })),
        )
        .web_app(shell)
        .build()?;
    for path in ["/", "/p/abc", "/data/x", "/sign-in", "/rawx", "/apix"] {
        let reply = send(&gw, get(path, "")?).await?;
        assert_eq!(reply.text, "app shell", "{path}");
        assert!(
            reply
                .headers
                .get("content-security-policy")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|csp| csp.contains("script-src 'self'")),
            "{path}"
        );
    }
    let api = send(&gw, get("/api/v1/nope", "")?).await?;
    assert_eq!(api.status, StatusCode::NOT_FOUND);
    assert_eq!(
        api.body.get("code"),
        Some(&json!("not_found")),
        "{}",
        api.text
    );
    for path in [
        "/raw",
        "/raw/unknown",
        "/healthz/x",
        "/api",
        "/collab",
        "/collab/s/d",
    ] {
        let reply = send(&gw, get(path, "")?).await?;
        assert_ne!(reply.text, "app shell", "{path}");
    }
    assert_eq!(
        send(&gw, get("/healthz", "")?).await?.status,
        StatusCode::OK
    );
    // Published pages on the public host never get the app.
    let public = Request::get("/p/abc")
        .header("host", "pages.northstar.dev")
        .body(Body::empty())?;
    let reply = send(&gw, public).await?;
    assert_ne!(reply.text, "app shell");
    assert!(reply.text.contains("page not found"));
    Ok(())
}

#[tokio::test]
async fn invalid_registrations_are_refused() {
    struct Twice;
    impl Service for Twice {
        fn name(&self) -> &'static str {
            "twice"
        }
        fn operations(self: Arc<Self>) -> Vec<Operation> {
            let op = || {
                Operation::query(
                    "twice.get",
                    "/v1/twice",
                    Scope::Read,
                    |_c, _i: Empty| async { Ok(Count { created: 0 }) },
                )
            };
            vec![op(), op()]
        }
    }
    let built = GatewayBuilder::new(config(), "t")
        .service(Arc::new(Twice))
        .build();
    assert!(built.is_err());
    let built = GatewayBuilder::new(config(), "t")
        .service(Arc::new(Notes {
            created: AtomicUsize::new(0),
            ready: Readiness::Ready,
        }))
        .service(Arc::new(Notes {
            created: AtomicUsize::new(0),
            ready: Readiness::Ready,
        }))
        .build();
    assert!(built.is_err(), "duplicate service names");
}
