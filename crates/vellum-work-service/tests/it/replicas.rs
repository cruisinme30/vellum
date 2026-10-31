// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Several replicas over one workspace: two `WorkService`s share one store,
//! the way two pods share a bucket. (One `LocalStore` stands in for the
//! bucket: like S3, it applies a conditional write atomically. Two
//! `LocalStore`s on one folder would not, which is why a multi-replica
//! deployment uses object storage.) Commands on both sides never hand out
//! the same issue number, and each side sees the other's writes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test fixtures fail loudly"
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use time::macros::datetime;
use vellum_contracts::{
    CredentialKind, Operation, Principal, RequestContext, ScopeSet, Service,
};
use vellum_kernel::{RequestId, Timestamp, UserId};
use vellum_store::LocalStore;
use vellum_work::UserRef;
use vellum_work_service::{
    EventLog, FixedClock, READ_FRESH, StaticDirectory, WorkService,
};

const PEOPLE: [&str; 2] = ["mara", "dev"];

#[derive(Clone)]
struct Replica {
    ops: Arc<Vec<Operation>>,
    users: Vec<(&'static str, UserId)>,
}

async fn replica(
    store: Arc<LocalStore>,
    users: &[(&'static str, UserId)],
) -> Replica {
    let mut directory = StaticDirectory::new();
    for (name, id) in users {
        directory = directory.with(*id, UserRef::parse(name).unwrap());
    }
    let (svc, _) = WorkService::open(
        store,
        Arc::new(directory),
        Arc::new(EventLog::new()),
        Arc::new(FixedClock(datetime!(2026-11-02 09:00 UTC))),
    )
    .await
    .unwrap();
    Replica {
        ops: Arc::new(Arc::new(svc).operations()),
        users: users.to_vec(),
    }
}

impl Replica {
    async fn call(&self, who: &str, op: &str, input: Value) -> Value {
        self.try_call(who, op, input.clone())
            .await
            .unwrap_or_else(|e| panic!("{op} as {who} failed: {e}"))
    }

    async fn try_call(
        &self,
        who: &str,
        op: &str,
        input: Value,
    ) -> Result<Value, String> {
        let id = self.users.iter().find(|(n, _)| *n == who).unwrap().1;
        let principal = Principal::user(
            id,
            CredentialKind::Session,
            ScopeSet::FULL,
            Timestamp::now(),
        );
        let found = self.ops.iter().find(|o| o.id() == op).unwrap();
        let ctx = RequestContext {
            request_id: RequestId::generate(),
            principal,
            deadline: Instant::now() + Duration::from_secs(30),
            traceparent: None,
        };
        found.call(ctx, input).await.map_err(|e| e.to_string())
    }

    async fn keys(&self) -> BTreeSet<String> {
        let out = self
            .call(
                "dev",
                "work.query_issues",
                json!({"team": "ENG", "limit": 1000}),
            )
            .await;
        out["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["issues"].as_array().unwrap().iter())
            .map(|i| i["key"].as_str().unwrap().to_owned())
            .collect()
    }
}

async fn pair() -> (tempfile::TempDir, Replica, Replica) {
    let dir = tempfile::tempdir().unwrap();
    let users: Vec<_> =
        PEOPLE.iter().map(|n| (*n, UserId::generate())).collect();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let a = replica(Arc::clone(&store), &users).await;
    let b = replica(store, &users).await;
    (dir, a, b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_creates_on_two_replicas_never_share_a_number() {
    let (_dir, a, b) = pair().await;
    let _ = a
        .call(
            "mara",
            "work.create_team",
            json!({
                "key": "ENG",
                "name": "Engineering",
                "lead": "mara",
                "members": ["dev"]
            }),
        )
        .await;
    let per_side = 15;
    let mut tasks = Vec::new();
    for (side, r) in [("a", a.clone()), ("b", b.clone())] {
        for n in 0..per_side {
            let r = r.clone();
            tasks.push(tokio::spawn(async move {
                let out = r
                    .call(
                        "dev",
                        "work.create_issue",
                        json!({"team": "ENG", "title": format!("{side} {n}")}),
                    )
                    .await;
                out["issue"]["key"].as_str().unwrap().to_owned()
            }));
        }
    }
    let mut created = BTreeSet::new();
    for t in tasks {
        assert!(
            created.insert(t.await.unwrap()),
            "a key was handed out twice"
        );
    }
    assert_eq!(created.len(), 2 * per_side);
    let expected: BTreeSet<String> =
        (1..=2 * per_side).map(|n| format!("ENG-{n}")).collect();
    assert_eq!(created, expected);

    tokio::time::sleep(READ_FRESH + Duration::from_millis(50)).await;
    assert_eq!(a.keys().await, expected);
    assert_eq!(b.keys().await, expected);
}

#[tokio::test]
async fn a_write_on_one_replica_is_read_on_the_other_within_a_second() {
    let (_dir, a, b) = pair().await;
    let _ = a
        .call(
            "mara",
            "work.create_team",
            json!({
                "key": "ENG",
                "name": "Engineering",
                "lead": "mara",
                "members": ["dev"]
            }),
        )
        .await;
    let _ = b
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Rate limits drop bursts"}),
        )
        .await;
    let _ = b
        .call(
            "dev",
            "work.add_comment",
            json!({"target": "ENG-1", "body": "Seen on the gateway."}),
        )
        .await;
    let _ = b
        .call(
            "dev",
            "work.update_issue",
            json!({"key": "ENG-1", "state": "in-progress"}),
        )
        .await;

    let start = Instant::now();
    loop {
        let issue = a
            .try_call("dev", "work.get_issue", json!({"key": "ENG-1"}))
            .await;
        let threads = a
            .try_call("dev", "work.list_comments", json!({"target": "ENG-1"}))
            .await;
        if let (Ok(issue), Ok(threads)) = (&issue, &threads)
            && issue["state"] == "in-progress"
            && threads["threads"].as_array().is_some_and(|t| t.len() == 1)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "replica a is still stale: {issue:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // A command on `a` runs on the state `b` wrote: the next number is 2.
    let next = a
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Retry storms"}),
        )
        .await;
    assert_eq!(next["issue"]["key"], "ENG-2");
}

#[tokio::test]
async fn a_new_team_on_one_replica_rebuilds_the_other() {
    let (_dir, a, b) = pair().await;
    let _ = a
        .call(
            "mara",
            "work.create_team",
            json!({
                "key": "OPS",
                "name": "Operations",
                "lead": "mara",
                "members": ["dev"]
            }),
        )
        .await;
    let out = b
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "OPS", "title": "Pager rotation"}),
        )
        .await;
    assert_eq!(out["issue"]["key"], "OPS-1");
}
