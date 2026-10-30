// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Search's operations through the real gateway: REST routes, the OpenAPI
//! document and `GraphQL`, with the real permission engine deciding what
//! each person sees.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_authz::{Actor, Authz, Change, Id, MemoryStore, Object, Relation, Subject, Tuple};
use vellum_contracts::{BoxFuture, CredentialKind, Principal, Scope, ScopeSet, Service};
use vellum_kernel::{Error, ErrorKind, SpaceId, Timestamp};
use vellum_search::conformance::{Corpus, StaticStars, bid};
use vellum_search::{Location, SearchApi, SearchService, TantivyBackend, TantivyConfig, Update};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const HOST: &str = "vellum.northstar.dev";

/// `Bearer ada` and `Bearer grace` (read scope); no header is anonymous.
struct Tokens(Corpus);

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
        let user = |id| {
            Ok(Authenticated {
                principal: Principal::user(
                    id,
                    CredentialKind::ApiToken,
                    ScopeSet::from_scopes([Scope::Read]),
                    Timestamp::now(),
                ),
                csrf_binding: None,
            })
        };
        let result = match header {
            "Bearer ada" => user(self.0.ada),
            "Bearer grace" => user(self.0.grace),
            "" => Ok(Authenticated::anonymous()),
            _ => Err(Error::new(ErrorKind::Unauthenticated, "invalid token")),
        };
        Box::pin(async move { result })
    }
}

/// The corpus's folder trees in the permission engine, Ada reading both
/// spaces and Grace reading engineering only.
async fn permissions(
    corpus: &Corpus,
) -> Result<Arc<Authz<MemoryStore>>, Box<dyn std::error::Error + Send + Sync>> {
    let store = MemoryStore::new();
    let mut trees: BTreeMap<SpaceId, BTreeMap<Id, Option<Id>>> = BTreeMap::new();
    for doc in corpus.documents() {
        let tree = trees.entry(doc.space).or_default();
        let space = doc.space.as_u128();
        let mut parent = None;
        for ancestor in doc.ancestors.iter().filter(|a| a.as_u128() != space) {
            let id = Id(ancestor.as_u128());
            let _ = tree.insert(id, parent);
            parent = Some(id);
        }
        let _ = tree.insert(Id::from(doc.doc), parent);
    }
    for (space, tree) in trees {
        store.set_tree(Id::from(space), tree)?;
    }
    let authz = Arc::new(Authz::new(store));
    let viewer = |space: SpaceId, user| {
        Change::Grant(Tuple::role(
            Object::Space(Id::from(space)),
            Relation::Viewer,
            Subject::User(Id::from(user)),
        ))
    };
    for (space, user) in [
        (corpus.eng, corpus.ada),
        (corpus.finance, corpus.ada),
        (corpus.eng, corpus.grace),
    ] {
        let _ = authz
            .write(Id::from(space), Actor::System, &[viewer(space, user)])
            .await?;
    }
    Ok(authz)
}

async fn gateway(corpus: &Corpus) -> Result<Gateway, Box<dyn std::error::Error + Send + Sync>> {
    let (backend, _) = TantivyBackend::open(&TantivyConfig {
        location: Location::Memory,
        key: None,
        model: None,
    })?;
    let service = SearchService::new(
        Arc::new(backend),
        permissions(corpus).await?,
        Arc::new(StaticStars::default()),
    );
    service
        .write(corpus.documents().into_iter().map(Update::Upsert).collect())
        .await?;
    Ok(GatewayBuilder::new(
        GatewayConfig::new("https://vellum.northstar.dev", vec![7; 32]),
        "1.0.0-test",
    )
    .service(Arc::new(SearchApi::new(service)) as Arc<dyn Service>)
    .authenticator(Arc::new(Tokens(*corpus)))
    .build()?)
}

async fn call(
    gateway: &Gateway,
    method: &str,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error + Send + Sync>> {
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
    let response = gateway.router().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

fn docs(page: &Value) -> Vec<String> {
    page["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|hit| hit["doc"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn every_operation_is_in_openapi_and_graphql() -> TestResult {
    let corpus = Corpus::default();
    let gateway = gateway(&corpus).await?;
    let (status, openapi) = call(&gateway, "GET", "/api/openapi.json", "", None).await?;
    assert_eq!(status, StatusCode::OK);
    let mut ids: Vec<String> = openapi["paths"]
        .as_object()
        .ok_or("no paths")?
        .values()
        .flat_map(|item| item.as_object().into_iter().flat_map(|m| m.values()))
        .filter_map(|op| op["operationId"].as_str().map(str::to_owned))
        .filter(|id| id.starts_with("search"))
        .collect();
    ids.sort();
    assert_eq!(ids, ["searchContext", "searchQuery", "searchTitles"]);
    let sdl = gateway.graphql_sdl();
    for needle in ["searchQuery(", "searchTitles(", "searchContext("] {
        assert!(sdl.contains(needle), "SDL lacks {needle}");
    }
    Ok(())
}

#[tokio::test]
async fn rest_search_is_filtered_by_permissions() -> TestResult {
    let corpus = Corpus::default();
    let gateway = gateway(&corpus).await?;

    let (status, ada) = call(&gateway, "GET", "/api/v1/search?q=budget", "ada", None).await?;
    assert_eq!(status, StatusCode::OK, "{ada}");
    assert!(docs(&ada).contains(&corpus.budget.to_string()), "{ada}");

    // Grace cannot read finance, so its budget page never shows up.
    let (status, grace) = call(&gateway, "GET", "/api/v1/search?q=budget", "grace", None).await?;
    assert_eq!(status, StatusCode::OK, "{grace}");
    assert!(
        !docs(&grace).contains(&corpus.budget.to_string()),
        "{grace}"
    );

    let (status, page) = call(
        &gateway,
        "GET",
        "/api/v1/search?q=canary%20deploy&kinds=page",
        "grace",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page["hits"][0]["block"],
        json!(bid(3).to_string()),
        "{page}"
    );

    let (status, _) = call(&gateway, "GET", "/api/v1/search?q=budget", "", None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn titles_and_context_over_rest() -> TestResult {
    let corpus = Corpus::default();
    let gateway = gateway(&corpus).await?;

    let (status, titles) = call(
        &gateway,
        "GET",
        "/api/v1/search/titles?q=gatew",
        "ada",
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{titles}");
    assert!(
        titles["hits"].as_array().is_some_and(|t| !t.is_empty()),
        "{titles}"
    );

    let uri = format!("/api/v1/search/context/{}/{}", corpus.runbook, bid(5));
    let (status, context) = call(&gateway, "GET", &uri, "ada", None).await?;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(
        context["section"].as_array().map(Vec::len),
        Some(3),
        "{context}"
    );

    let missing = format!("/api/v1/search/context/{}/{}", corpus.runbook, bid(77));
    let (status, _) = call(&gateway, "GET", &missing, "ada", None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn graphql_searches() -> TestResult {
    let corpus = Corpus::default();
    let gateway = gateway(&corpus).await?;
    let query = json!({
        "query": r#"{ searchQuery(input: {q: "canary deploy", kinds: "page"}) { hits { doc block citation } has_more } }"#
    });
    let (status, reply) = call(&gateway, "POST", "/api/graphql", "grace", Some(query)).await?;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(
        reply.pointer("/data/searchQuery/hits/0/block"),
        Some(&json!(bid(3).to_string())),
        "{reply}"
    );
    let titles =
        json!({ "query": r#"{ searchTitles(input: {q: "gatew"}) { hits { doc title } } }"# });
    let (status, reply) = call(&gateway, "POST", "/api/graphql", "ada", Some(titles)).await?;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(
        reply
            .pointer("/data/searchTitles/hits")
            .and_then(Value::as_array)
            .is_some_and(|h| !h.is_empty()),
        "{reply}"
    );
    Ok(())
}
