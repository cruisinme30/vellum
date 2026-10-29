// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive's operations through the real gateway: REST routes, the
//! OpenAPI document and `GraphQL`, on a real folder with the real
//! permission engine.

#![allow(clippy::too_many_lines, reason = "one test walks a page's whole life")]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_authz::{Authz, MemoryStore};
use vellum_contracts::{BoxFuture, CredentialKind, Principal, Scope, ScopeSet, Service};
use vellum_drive::authz::AuthzAccess;
use vellum_drive::memory::MemoryIndex;
use vellum_drive::{Drive, DriveService};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const HOST: &str = "vellum.northstar.dev";
const ALICE: UserId = UserId::from_u128(1);
const BOB: UserId = UserId::from_u128(2);

/// `Bearer alice`, `Bearer bob` (full scopes), `Bearer alice-read`
/// (read only), `Bearer alice-stale` (full scopes, signed in an hour
/// ago); no header is anonymous.
struct Tokens;

impl Authenticator for Tokens {
    fn authenticate<'a>(
        &'a self,
        request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>> {
        let header = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let signed_in_at = |id, scopes, at| {
            Ok(Authenticated {
                principal: Principal::user(id, CredentialKind::ApiToken, scopes, at),
                csrf_binding: None,
            })
        };
        let user = |id, scopes| signed_in_at(id, scopes, Timestamp::now());
        let an_hour_ago = Timestamp::from_unix_ms(Timestamp::now().unix_ms() - 3_600_000)
            .unwrap_or_else(|_| Timestamp::now());
        let result = match header {
            "Bearer alice" => user(ALICE, ScopeSet::FULL),
            "Bearer bob" => user(BOB, ScopeSet::FULL),
            "Bearer alice-read" => user(ALICE, ScopeSet::from_scopes([Scope::Read])),
            "Bearer alice-stale" => signed_in_at(ALICE, ScopeSet::FULL, an_hour_ago),
            "" => Ok(Authenticated::anonymous()),
            _ => Err(Error::new(ErrorKind::Unauthenticated, "invalid token")),
        };
        Box::pin(async move { result })
    }
}

struct World {
    _dir: tempfile::TempDir,
    gateway: Gateway,
}

async fn world() -> Result<World, Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = Arc::new(MemoryIndex::new());
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let access = Arc::new(AuthzAccess::new(authz, index.clone()));
    let drive = Arc::new(Drive::new(store, index, access));
    let gateway = GatewayBuilder::new(
        GatewayConfig::new("https://vellum.northstar.dev", vec![7; 32]),
        "1.0.0-test",
    )
    .service(Arc::new(DriveService::new(drive)) as Arc<dyn Service>)
    .authenticator(Arc::new(Tokens))
    .build()?;
    Ok(World { _dir: dir, gateway })
}

struct Reply {
    status: StatusCode,
    body: Value,
    text: String,
}

async fn call(
    w: &World,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> Result<Reply, Box<dyn std::error::Error + Send + Sync>> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST);
    if !auth.is_empty() {
        b = b.header("authorization", format!("Bearer {auth}"));
    }
    let request = match body {
        Some(json) => b
            .header("content-type", "application/json")
            .body(Body::from(json.to_string()))?,
        None => b.body(Body::empty())?,
    };
    let response = w.gateway.router().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        text,
    })
}

async fn ok(
    w: &World,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let reply = call(w, method, uri, auth, body).await?;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{method} {uri}: {}",
        reply.text
    );
    Ok(reply.body)
}

fn field(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn every_operation_is_in_openapi_and_graphql() -> TestResult {
    let w = world().await?;
    let openapi = ok(&w, "GET", "/api/openapi.json", "", None).await?;
    let ids: Vec<String> = openapi["paths"]
        .as_object()
        .ok_or("no paths")?
        .values()
        .flat_map(|item| item.as_object().into_iter().flat_map(|m| m.values()))
        .filter_map(|op| op["operationId"].as_str().map(str::to_owned))
        .filter(|id| id.starts_with("drive"))
        .collect();
    assert_eq!(ids.len(), 37, "{ids:?}");
    let sdl = w.gateway.graphql_sdl();
    for needle in ["driveGet(", "driveCreatePage(", "driveFind(", "driveSave("] {
        assert!(sdl.contains(needle), "SDL lacks {needle}");
    }
    Ok(())
}

/// Admins manage every space; others cannot, and an old sign-in is asked
/// to confirm again first (R-SE-7, D241-D242).
#[tokio::test]
async fn admins_manage_spaces_over_rest() -> TestResult {
    let w = world().await?;
    let made = ok(
        &w,
        "POST",
        "/api/v1/admin/spaces",
        "alice",
        Some(json!({"slug": "ops", "name": "Operations", "owner_id": BOB.to_string()})),
    )
    .await?;
    let space_id = field(&made, "/space/id");
    assert_eq!(field(&made, "/members/0/role"), "owner");
    assert_eq!(field(&made, "/members/0/user_id"), BOB.to_string());

    let added = ok(
        &w,
        "PUT",
        &format!("/api/v1/admin/spaces/{space_id}/members/{ALICE}"),
        "alice",
        Some(json!({"role": "viewer"})),
    )
    .await?;
    assert_eq!(added["members"].as_array().map(Vec::len), Some(2));
    let listed = ok(&w, "GET", "/api/v1/admin/spaces", "alice", None).await?;
    assert_eq!(field(&listed, "/spaces/0/space/name"), "Operations");

    let reader = call(&w, "GET", "/api/v1/admin/spaces", "alice-read", None).await?;
    assert_eq!(reader.status, StatusCode::FORBIDDEN, "{}", reader.text);
    let stale = call(
        &w,
        "PUT",
        &format!("/api/v1/admin/spaces/{space_id}/members/{ALICE}"),
        "alice-stale",
        Some(json!({"role": "owner"})),
    )
    .await?;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED, "{}", stale.text);
    assert_eq!(field(&stale.body, "/message"), "reauthentication required");
    Ok(())
}

#[tokio::test]
async fn a_page_lives_through_rest() -> TestResult {
    let w = world().await?;
    let space = ok(
        &w,
        "POST",
        "/api/v1/drive/spaces",
        "alice",
        Some(json!({"kind": "shared_drive", "slug": "eng", "name": "Engineering"})),
    )
    .await?;
    let space_id = field(&space, "/id");
    let folder = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/spaces/{space_id}/folders"),
        "alice",
        Some(json!({"name": "Plans"})),
    )
    .await?;
    let folder_id = field(&folder, "/id");
    let page = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/spaces/{space_id}/pages"),
        "alice",
        Some(json!({"parent_id": folder_id, "title": "Roadmap", "body": "Q1: ship"})),
    )
    .await?;
    let page_id = field(&page, "/id");
    assert_eq!(field(&page, "/kind"), "page");

    let items = ok(
        &w,
        "GET",
        &format!("/api/v1/drive/spaces/{space_id}/items?parent_id={folder_id}"),
        "alice",
        None,
    )
    .await?;
    assert_eq!(field(&items, "/items/0/title"), "Roadmap");

    // Read, then save on top of what was read.
    let read = ok(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}/content"),
        "alice",
        None,
    )
    .await?;
    let base = field(&read, "/content");
    assert!(base.contains("Q1: ship"), "{base}");
    let saved = ok(
        &w,
        "PUT",
        &format!("/api/v1/drive/items/{page_id}/content"),
        "alice",
        Some(json!({
            "base_content": base,
            "base_etag": field(&read, "/etag"),
            "content": base.replace("Q1: ship", "Q1: ship it"),
        })),
    )
    .await?;
    assert_eq!(field(&saved, "/result"), "written", "{saved}");
    let versions = ok(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}/versions"),
        "alice",
        None,
    )
    .await?;
    assert!(
        !versions["versions"]
            .as_array()
            .ok_or("no versions")?
            .is_empty()
    );

    // Rename, find, star.
    let renamed = ok(
        &w,
        "PATCH",
        &format!("/api/v1/drive/items/{page_id}"),
        "alice",
        Some(json!({"title": "Roadmap 2027"})),
    )
    .await?;
    assert_eq!(field(&renamed, "/title"), "Roadmap 2027");
    let found = ok(
        &w,
        "GET",
        "/api/v1/drive/search?title=2027&kinds=page",
        "alice",
        None,
    )
    .await?;
    assert_eq!(field(&found, "/items/0/id"), page_id);
    let _ = ok(
        &w,
        "PUT",
        &format!("/api/v1/drive/items/{page_id}/star"),
        "alice",
        Some(json!({"starred": true})),
    )
    .await?;
    let starred = ok(&w, "GET", "/api/v1/drive/starred", "alice", None).await?;
    assert_eq!(field(&starred, "/items/0/id"), page_id);

    // Trash, restore, trash again and purge.
    let _ = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/items/{folder_id}/trash"),
        "alice",
        None,
    )
    .await?;
    let trash = ok(
        &w,
        "GET",
        &format!("/api/v1/drive/spaces/{space_id}/trash"),
        "alice",
        None,
    )
    .await?;
    assert!(trash.to_string().contains(&folder_id), "{trash}");
    let restored = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/items/{folder_id}/restore"),
        "alice",
        None,
    )
    .await?;
    assert_eq!(restored["trashed_at"], Value::Null);
    let _ = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/items/{page_id}/trash"),
        "alice",
        None,
    )
    .await?;
    let purged = ok(
        &w,
        "DELETE",
        &format!("/api/v1/drive/items/{page_id}"),
        "alice",
        None,
    )
    .await?;
    assert_eq!(purged, json!({"ok": true}));
    let gone = call(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}"),
        "alice",
        None,
    )
    .await?;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);
    Ok(())
}

#[tokio::test]
async fn the_drive_and_the_scope_both_decide() -> TestResult {
    let w = world().await?;
    let space = ok(
        &w,
        "POST",
        "/api/v1/drive/spaces",
        "alice",
        Some(json!({"kind": "shared_drive", "slug": "eng", "name": "Engineering"})),
    )
    .await?;
    let space_id = field(&space, "/id");
    let page = ok(
        &w,
        "POST",
        &format!("/api/v1/drive/spaces/{space_id}/pages"),
        "alice",
        Some(json!({"title": "Secret"})),
    )
    .await?;
    let page_id = field(&page, "/id");

    // Bob has no access to Alice's drive; anonymous callers have none.
    let denied = call(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}"),
        "bob",
        None,
    )
    .await?;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);
    let anonymous = call(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}"),
        "",
        None,
    )
    .await?;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text
    );

    // A read-only token reads but cannot change anything.
    let _ = ok(
        &w,
        "GET",
        &format!("/api/v1/drive/items/{page_id}"),
        "alice-read",
        None,
    )
    .await?;
    let blocked = call(
        &w,
        "PATCH",
        &format!("/api/v1/drive/items/{page_id}"),
        "alice-read",
        Some(json!({"title": "Renamed"})),
    )
    .await?;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN, "{}", blocked.text);

    // A My Drive always belongs to the caller.
    let mine = ok(
        &w,
        "POST",
        "/api/v1/drive/spaces",
        "bob",
        Some(json!({"kind": "user_drive", "slug": "bob", "name": "Bob"})),
    )
    .await?;
    assert_eq!(field(&mine, "/owner"), BOB.to_string());
    let my_drive = ok(&w, "GET", "/api/v1/drive/my-drive", "bob", None).await?;
    assert_eq!(field(&my_drive, "/space/id"), field(&mine, "/id"));
    Ok(())
}

#[tokio::test]
async fn graphql_reads_and_writes_the_drive() -> TestResult {
    let w = world().await?;
    let gql = |query: String| Some(json!({ "query": query }));
    let space = ok(
        &w,
        "POST",
        "/api/graphql",
        "alice",
        gql(r#"mutation { driveCreateSpace(input: {kind: shared_drive, slug: "eng", name: "Engineering"}) { id name } }"#.to_owned()),
    )
    .await?;
    let space_id = field(&space, "/data/driveCreateSpace/id");
    assert!(!space_id.is_empty(), "{space}");
    let page = ok(
        &w,
        "POST",
        "/api/graphql",
        "alice",
        gql(format!(
            r#"mutation {{ driveCreatePage(input: {{space_id: "{space_id}", title: "Notes", body: "hi"}}) {{ id title kind }} }}"#
        )),
    )
    .await?;
    let page_id = field(&page, "/data/driveCreatePage/id");
    assert_eq!(field(&page, "/data/driveCreatePage/kind"), "page", "{page}");
    let read = ok(
        &w,
        "POST",
        "/api/graphql",
        "alice",
        gql(format!(
            r#"{{ driveRead(input: {{id: "{page_id}"}}) {{ content item {{ title }} }} }}"#
        )),
    )
    .await?;
    assert!(
        field(&read, "/data/driveRead/content").contains("hi"),
        "{read}"
    );
    assert_eq!(field(&read, "/data/driveRead/item/title"), "Notes");
    Ok(())
}
