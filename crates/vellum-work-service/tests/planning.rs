// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Planning end to end (D223, D224): projects, milestones, health updates,
//! initiatives, templates and recurring issues, through the operations and
//! back from disk after a restart.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "test fixtures fail loudly"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use time::OffsetDateTime;
use time::macros::datetime;
use vellum_contracts::{CredentialKind, Operation, Principal, RequestContext, ScopeSet, Service};
use vellum_kernel::{ErrorKind, RequestId, Timestamp, UserId};
use vellum_store::LocalStore;
use vellum_work::UserRef;
use vellum_work_service::{EventLog, FixedClock, StaticDirectory, WorkService};

struct World {
    dir: tempfile::TempDir,
    svc: Arc<WorkService>,
    ops: Vec<Operation>,
    users: Vec<(&'static str, UserId)>,
}

async fn open(
    dir: tempfile::TempDir,
    users: Vec<(&'static str, UserId)>,
    now: OffsetDateTime,
) -> World {
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let mut directory = StaticDirectory::new();
    for (name, id) in &users {
        directory = directory.with(*id, UserRef::parse(name).unwrap());
    }
    let (svc, report) = WorkService::open(
        store,
        Arc::new(directory),
        Arc::new(EventLog::new()),
        Arc::new(FixedClock(now)),
    )
    .await
    .unwrap();
    assert_eq!(report.skipped, 0);
    let svc = Arc::new(svc);
    let ops = Arc::clone(&svc).operations();
    World {
        dir,
        svc,
        ops,
        users,
    }
}

impl World {
    async fn try_call(&self, who: &str, op: &str, input: Value) -> Result<Value, ErrorKind> {
        let id = self.users.iter().find(|(n, _)| *n == who).unwrap().1;
        let principal = Principal::user(
            id,
            CredentialKind::Session,
            ScopeSet::FULL,
            Timestamp::now(),
        );
        let op = self
            .ops
            .iter()
            .find(|o| o.id() == op)
            .unwrap_or_else(|| panic!("no {op}"));
        let ctx = RequestContext {
            request_id: RequestId::generate(),
            principal,
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        };
        op.call(ctx, input).await.map_err(|e| e.kind())
    }

    async fn call(&self, who: &str, op: &str, input: Value) -> Value {
        self.try_call(who, op, input)
            .await
            .unwrap_or_else(|e| panic!("{op} failed: {e:?}"))
    }

    async fn reopen(self, now: OffsetDateTime) -> Self {
        open(self.dir, self.users, now).await
    }
}

async fn world() -> World {
    let users = vec![("mara", UserId::generate()), ("dev", UserId::generate())];
    let w = open(
        tempfile::tempdir().unwrap(),
        users,
        datetime!(2026-11-02 09:00 UTC),
    )
    .await;
    let _ = w
        .call(
            "mara",
            "work.create_team",
            json!({"key": "ENG", "name": "Engineering", "lead": "mara", "members": ["dev"]}),
        )
        .await;
    w
}

#[tokio::test]
async fn projects_milestones_and_updates_survive_a_restart() {
    let w = world().await;
    let p = w
        .call("mara", "work.create_project", json!({"title": "Gateway replay", "teams": ["ENG"], "lead": "@mara", "status": "in-progress", "target_date": "2026-12-01"}))
        .await;
    let id = p["id"].as_str().unwrap().to_owned();
    assert_eq!(p["status"], "in-progress");
    assert_eq!(p["health"], Value::Null);
    assert_eq!(
        p["update_due"], true,
        "an active project with no update owes one"
    );

    let p = w
        .call(
            "mara",
            "work.add_milestone",
            json!({"project_id": id, "name": "Beta", "target_date": "2026-11-15"}),
        )
        .await;
    let milestone = p["milestones"][0]["id"].as_str().unwrap().to_owned();

    let issue = w
        .call("dev", "work.create_issue", json!({"team": "ENG", "title": "Record traffic", "project": id, "milestone": milestone, "estimate": "3"}))
        .await;
    let key = issue["issue"]["key"].as_str().unwrap().to_owned();
    let _ = w
        .call(
            "dev",
            "work.update_issue",
            json!({"key": key, "state": "done"}),
        )
        .await;

    let posted = w
        .call(
            "mara",
            "work.post_update",
            json!({"project_id": id, "health": "at-risk", "body": "Replay is slow."}),
        )
        .await;
    assert_eq!(posted["author"], "mara");
    assert_eq!(
        w.try_call(
            "mara",
            "work.post_update",
            json!({"project_id": id, "health": "fine"})
        )
        .await,
        Err(ErrorKind::InvalidInput)
    );

    let ini = w
        .call(
            "mara",
            "work.create_initiative",
            json!({"title": "Q4 launch", "projects": [id]}),
        )
        .await;
    assert_eq!(ini["health"], "at-risk");
    assert_eq!(ini["percent"], 100);

    let w = w.reopen(datetime!(2026-11-03 09:00 UTC)).await;
    let p = w
        .call("dev", "work.get_project", json!({"project_id": id}))
        .await;
    assert_eq!(p["health"], "at-risk");
    assert_eq!(p["update_due"], false);
    assert_eq!(p["milestones"][0]["name"], "Beta");
    assert_eq!(p["milestones"][0]["percent"], 100);
    let updates = w
        .call("dev", "work.list_updates", json!({"project_id": id}))
        .await;
    assert_eq!(updates["updates"][0]["body"], "Replay is slow.\n");
    let list = w.call("dev", "work.list_initiatives", json!({})).await;
    assert_eq!(list["initiatives"][0]["title"], "Q4 launch");
    assert_eq!(list["initiatives"][0]["projects"][0]["id"], id.as_str());

    // An initiative that names a missing project is refused.
    assert_eq!(
        w.try_call(
            "mara",
            "work.create_initiative",
            json!({"title": "Ghost", "projects": ["01JBZ3M8K2Q4R5S6T7V8W9X0P1"]})
        )
        .await,
        Err(ErrorKind::NotFound)
    );
}

#[tokio::test]
async fn a_repeating_issue_comes_back_after_its_due_date() {
    let w = world().await;
    let created = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Rotate keys", "due": "2026-11-02", "repeat": "1w"}),
        )
        .await;
    assert_eq!(created["issue"]["repeat"], "1w");
    assert_eq!(
        w.try_call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "No due", "repeat": "1w"})
        )
        .await,
        Err(ErrorKind::InvalidInput)
    );
    assert_eq!(
        w.try_call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Bad", "due": "2026-11-02", "repeat": "often"})
        )
        .await,
        Err(ErrorKind::InvalidInput)
    );

    // The next day's run files ENG-2, due a week later.
    let w = w.reopen(datetime!(2026-11-03 00:01 UTC)).await;
    w.svc.run_daily().await.unwrap();
    let next = w
        .call("dev", "work.get_issue", json!({"key": "ENG-2"}))
        .await;
    assert_eq!(next["title"], "Rotate keys");
    assert_eq!(next["due"], "2026-11-09");
    assert_eq!(next["repeat"], "1w");
    let old = w
        .call("dev", "work.get_issue", json!({"key": "ENG-1"}))
        .await;
    assert_eq!(old["repeat"], Value::Null);

    // Stopping a repeat: an empty string clears it.
    let stopped = w
        .call(
            "dev",
            "work.update_issue",
            json!({"key": "ENG-2", "repeat": ""}),
        )
        .await;
    assert_eq!(stopped["repeat"], Value::Null);
    let templates = w
        .call("dev", "work.list_templates", json!({"team": "ENG"}))
        .await;
    assert!(templates["templates"].is_array());
}
