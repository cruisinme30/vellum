// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The issue import route over a real Work service (#151).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt as _;
use url::Url;
use vellum_contracts::{CredentialKind, Principal, Scope, ScopeSet};
use vellum_identity::{Identity, IdentityConfig, Store};
use vellum_kernel::{Timestamp, UserId};
use vellum_security::upload::{Scanner, UploadError, UploadPolicy};
use vellum_store::BlobStore;
use vellum_store::LocalStore;
use vellum_work::{Changes, Team, TeamKey, UserRef};
use vellum_work_service::{
    EventLog, StaticDirectory, SystemClock, WorkService,
};

use crate::issue_import::{PREFIX, router};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

struct NoVirus;

impl Scanner for NoVirus {
    async fn scan(&self, _bytes: &[u8]) -> Result<(), UploadError> {
        Ok(())
    }
}

const ISSUES: &str = r#"[
  {"number": 7, "title": "Crash on save", "body": "Steps to reproduce.",
    "state": "open",
   "created_at": "2026-10-01T09:00:00Z", "user": {"login": "dana"},
     "labels": [{"name": "bug"}]},
  {"number": 9, "title": "Ship dark mode", "body": "", "state": "closed",
    "state_reason": "completed",
   "created_at": "2026-10-02T09:00:00Z", "closed_at": "2026-10-05T09:00:00Z",
     "user": {"login": "lee"}},
  {"number": 10, "title": "Add search", "state": "open", "pull_request": {},
   "created_at": "2026-10-03T09:00:00Z"}
]"#;

struct World {
    _dir: tempfile::TempDir,
    work: Arc<WorkService>,
    identity: Arc<Identity>,
    admin: UserId,
    member: UserId,
}

impl World {
    async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dir = tempfile::tempdir()?;
        let store: Arc<dyn BlobStore> =
            Arc::new(LocalStore::open(dir.path()).await?);
        let (work, _) = WorkService::open(
            store,
            Arc::new(StaticDirectory::new()),
            Arc::new(EventLog::new()),
            Arc::new(SystemClock),
        )
        .await?;
        let work = Arc::new(work);
        let team = Team::new(
            "01JC00000000000000000000E1".parse()?,
            TeamKey::parse("ENG")?,
            "Engineering",
        )?;
        work.transact(UserRef::parse("admin")?, |graph, _, _| {
            graph.insert_team(team.clone())?;
            let mut changes = Changes::default();
            let _ = changes.teams.insert(TeamKey::parse("ENG")?);
            Ok((changes, None, ()))
        })
        .await?;
        let identity = Arc::new(Identity::new(
            IdentityConfig::new(Url::parse("https://vellum.northstar.dev")?),
            Store::memory(),
        ));
        let admin = identity
            .create_user("ana@northstar.dev", "Ana Admin", None, true)?
            .id;
        let member = identity
            .create_user("lee@northstar.dev", "Lee Member", None, false)?
            .id;
        Ok(Self {
            _dir: dir,
            work,
            identity,
            admin,
            member,
        })
    }

    fn app(&self, user: UserId, scopes: &[Scope]) -> Router {
        let principal = Principal::user(
            user,
            CredentialKind::Session,
            ScopeSet::from_scopes(scopes.iter().copied()),
            Timestamp::now(),
        );
        Router::new()
            .nest(
                PREFIX,
                router(
                    Arc::clone(&self.work),
                    Arc::clone(&self.identity),
                    Some(Arc::new(NoVirus)),
                    UploadPolicy::default(),
                ),
            )
            .layer(axum::Extension(principal))
    }
}

fn post(query: &str, body: &str) -> Request<Body> {
    Request::post(format!("{PREFIX}?{query}"))
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body.to_owned()))
        .unwrap_or_else(|e| unreachable!("{e}"))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|e| unreachable!("{e}"));
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const QUERY: &str =
    "tracker=github&team=ENG&repo=northstar/app&name=issues.json";

#[tokio::test]
async fn github_issues_import_once_and_skip_on_rerun() -> TestResult {
    let w = World::new().await?;
    let app = w.app(w.admin, &Scope::ALL);

    let (status, plan) =
        call(&app, post(&format!("{QUERY}&dry_run=true"), ISSUES)).await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(plan["created"], 2, "{plan}");
    let count = w.work.read(|graph, _| graph.issues().count()).await;
    assert_eq!(count, 0, "a dry run created issues");

    let (status, done) = call(&app, post(QUERY, ISSUES)).await;
    assert_eq!(status, StatusCode::CREATED, "{done}");
    assert_eq!(done["created"], 2);
    let titles: Vec<String> = w
        .work
        .read(|graph, _| graph.issues().map(|i| i.title.clone()).collect())
        .await;
    assert!(titles.contains(&"Crash on save".to_owned()), "{titles:?}");

    let (status, again) = call(&app, post(QUERY, ISSUES)).await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["created"], 0);
    assert_eq!(again["skipped"], 2);
    let count = w.work.read(|graph, _| graph.issues().count()).await;
    assert_eq!(count, 2);
    Ok(())
}

#[tokio::test]
async fn only_admins_with_the_work_scope_import() -> TestResult {
    let w = World::new().await?;
    let (status, _) =
        call(&w.app(w.member, &Scope::ALL), post(QUERY, ISSUES)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) =
        call(&w.app(w.admin, &[Scope::Read]), post(QUERY, ISSUES)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let app = w.app(w.admin, &Scope::ALL);
    let (status, _) =
        call(&app, post("tracker=asana&team=ENG&name=a.json", ISSUES)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) =
        call(&app, post("tracker=github&team=ENG&name=a.json", ISSUES)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "GitHub needs a repo");
    let (status, _) = call(&app, post(QUERY, "not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        post("tracker=github&team=NOPE&repo=a/b&name=a.json", ISSUES),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An extension that lies about the content is refused by the upload check.
    let (status, _) = call(
        &app,
        post("tracker=github&team=ENG&repo=a/b&name=a.png", ISSUES),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    Ok(())
}
