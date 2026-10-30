// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Work service end to end: operations called the way the gateway
//! calls them, files written to a real folder, and a restart that rebuilds
//! everything from those files.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::too_many_lines,
    reason = "test fixtures fail loudly"
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use time::macros::datetime;
use vellum_contracts::{
    CredentialKind, Operation, Principal, RequestContext, Scope, ScopeSet, Service,
};
use vellum_kernel::{ErrorKind, RequestId, Timestamp, UserId};
use vellum_store::LocalStore;
use vellum_work::{UserRef, WorkEvent};
use vellum_work_service::{EventLog, FixedClock, StaticDirectory, WorkService};

struct World {
    dir: tempfile::TempDir,
    ops: Vec<Operation>,
    events: Arc<EventLog>,
    users: Vec<(&'static str, UserId)>,
}

const PEOPLE: [&str; 3] = ["mara", "dev", "priya"];

async fn open(dir: tempfile::TempDir, users: Vec<(&'static str, UserId)>) -> (World, usize) {
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let mut directory = StaticDirectory::new();
    for (name, id) in &users {
        directory = directory.with(*id, UserRef::parse(name).unwrap());
    }
    let events = Arc::new(EventLog::new());
    let (svc, report) = WorkService::open(
        store,
        Arc::new(directory),
        events.clone(),
        Arc::new(FixedClock(datetime!(2026-11-02 09:00 UTC))),
    )
    .await
    .unwrap();
    let ops = Arc::new(svc).operations();
    (
        World {
            dir,
            ops,
            events,
            users,
        },
        report.skipped,
    )
}

async fn world() -> World {
    let users = PEOPLE.iter().map(|n| (*n, UserId::generate())).collect();
    open(tempfile::tempdir().unwrap(), users).await.0
}

impl World {
    fn principal(&self, who: &str, scopes: ScopeSet) -> Principal {
        let id = self
            .users
            .iter()
            .find(|(n, _)| *n == who)
            .map_or_else(UserId::generate, |(_, id)| *id);
        Principal::user(id, CredentialKind::Session, scopes, Timestamp::now())
    }

    async fn call_as(
        &self,
        principal: Principal,
        op: &str,
        input: Value,
    ) -> Result<Value, ErrorKind> {
        let op = self
            .ops
            .iter()
            .find(|o| o.id() == op)
            .unwrap_or_else(|| panic!("no operation {op}"));
        let ctx = RequestContext {
            request_id: RequestId::generate(),
            principal,
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        };
        op.call(ctx, input).await.map_err(|e| e.kind())
    }

    async fn call(&self, who: &str, op: &str, input: Value) -> Value {
        self.call_as(self.principal(who, ScopeSet::FULL), op, input)
            .await
            .unwrap_or_else(|e| panic!("{op} as {who} failed: {e:?}"))
    }

    async fn reopen(self) -> (Self, usize) {
        open(self.dir, self.users).await
    }
}

async fn with_eng(w: &World) {
    let _ = w
        .call("mara", "work.create_team", json!({"key": "ENG", "name": "Engineering", "lead": "mara", "members": ["dev", "priya"]}))
        .await;
}

#[tokio::test]
async fn every_operation_is_valid_and_unique() {
    let w = world().await;
    let mut ids = BTreeSet::new();
    let mut routes = BTreeSet::new();
    for op in &w.ops {
        op.validate("work")
            .unwrap_or_else(|e| panic!("{}: {e}", op.id()));
        assert!(ids.insert(op.id()), "duplicate id {}", op.id());
        assert!(
            routes.insert((op.method().as_str(), op.path())),
            "duplicate route {}",
            op.path()
        );
        assert!(!op.summary_text().is_empty(), "{} has no summary", op.id());
    }
    assert!(w.ops.len() >= 24, "{}", w.ops.len());
    let admin: Vec<&str> = w
        .ops
        .iter()
        .filter(|o| o.scope() == Scope::Admin)
        .map(Operation::id)
        .collect();
    assert_eq!(
        admin,
        [
            "work.create_team",
            "work.put_team",
            "work.archive_team",
            "work.restore_team"
        ]
    );
    assert!(
        w.ops
            .iter()
            .filter(|o| o.scope() == Scope::Admin)
            .all(|o| o.fresh_auth_window().is_some())
    );
}

#[tokio::test]
async fn issues_round_trip_through_the_api_and_the_disk() {
    let w = world().await;
    with_eng(&w).await;
    let team = w.call("dev", "work.get_team", json!({"team": "ENG"})).await;
    assert_eq!(team["default_status"], "backlog");

    let created = w
        .call(
            "dev",
            "work.create_issue",
            json!({
                "team": "ENG",
                "title": "Rate limits drop mobile bursts",
                "description": "Seen on the gateway.",
                "priority": 1,
                "assignee": "priya",
                "source_block": "01JB3K9Q7Z5N8M2X4V6T0R1Y3W#blk-bbbbbbbb",
                "quote": "Rate limits drop bursts"
            }),
        )
        .await;
    assert_eq!(created["issue"]["key"], "ENG-1");
    assert_eq!(created["chip"]["key"], "ENG-1");
    assert_eq!(created["issue"]["links"][0]["kind"], "source");
    assert_eq!(created["issue"]["sla"]["risk"], "gray");
    assert!(w.dir.path().join("teams/ENG/issues/ENG-1.md").exists());

    let _ = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Retry storms", "estimate": "3"}),
        )
        .await;
    let updated = w
        .call(
            "priya",
            "work.update_issue",
            json!({
                "key": "ENG-1",
                "state": "in-progress",
                "add_relations": [{"kind": "blocked_by", "issue": "ENG-2"}],
                "subscribe": true
            }),
        )
        .await;
    assert_eq!(updated["category"], "started");
    assert!(updated["started_at"].is_string());
    assert_eq!(updated["relations"][0]["issue"], "ENG-2");
    let other = w
        .call("dev", "work.get_issue", json!({"key": "ENG-2"}))
        .await;
    assert_eq!(
        other["relations"][0]["kind"], "blocks",
        "relations are two-way"
    );

    let mine = w
        .call(
            "priya",
            "work.query_issues",
            json!({"filter": r#"{"field":"assignee","op":"is","values":["@me"]}"#, "group_by": "status"}),
        )
        .await;
    let keys: Vec<&str> = mine["groups"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g["issues"].as_array().unwrap())
        .map(|i| i["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["ENG-1"]);
    let focus = w.call("priya", "work.my_issues", json!({})).await;
    assert_eq!(focus["issues"][0]["focus"], "urgent");

    // Moving keeps the old key working.
    let _ = w
        .call(
            "mara",
            "work.create_team",
            json!({"key": "OPS", "name": "Operations"}),
        )
        .await;
    let moved = w
        .call(
            "dev",
            "work.move_issue",
            json!({"key": "ENG-2", "to": "OPS"}),
        )
        .await;
    assert_eq!(moved["key"], "OPS-1");
    assert_eq!(
        w.call("dev", "work.get_issue", json!({"key": "ENG-2"}))
            .await["key"],
        "OPS-1"
    );
    assert!(!w.dir.path().join("teams/ENG/issues/ENG-2.md").exists());

    let events = w.events.take();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkEvent::IssueCreated { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, WorkEvent::IssueMoved { .. }))
    );

    // A restart rebuilds the same state from the files.
    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 0);
    let again = w
        .call("dev", "work.get_issue", json!({"key": "ENG-1"}))
        .await;
    assert_eq!(again["state"], "in-progress");
    assert_eq!(again["relations"][0]["issue"], "OPS-1");
    assert_eq!(
        w.call("dev", "work.get_issue", json!({"key": "ENG-2"}))
            .await["key"],
        "OPS-1"
    );
    let next = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Third"}),
        )
        .await;
    assert_eq!(next["issue"]["key"], "ENG-3", "numbers are never reused");
}

#[tokio::test]
async fn comments_assign_into_my_work_and_survive_a_restart() {
    let w = world().await;
    with_eng(&w).await;
    let _ = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Retry storms"}),
        )
        .await;

    let started = w
        .call(
            "priya",
            "work.add_comment",
            json!({"target": "ENG-1", "body": "@dev can you size this?", "assign_to": "dev"}),
        )
        .await;
    let thread = started["thread"]["id"].as_str().unwrap().to_owned();
    assert_eq!(started["thread"]["target"]["issue"], "ENG-1");
    let issue = w
        .call("dev", "work.get_issue", json!({"key": "ENG-1"}))
        .await;
    let subs: Vec<&str> = issue["subscribers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(subs.contains(&"priya") && subs.contains(&"dev"), "{subs:?}");

    let todo = w.call("dev", "work.my_assignments", json!({})).await;
    assert_eq!(todo["assignments"][0]["summary"], "@dev can you size this?");

    let reply = w
        .call(
            "dev",
            "work.add_comment",
            json!({"reply_to": thread, "body": "3 points"}),
        )
        .await;
    let reply_id = reply["thread"]["comments"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let _ = w
        .call(
            "priya",
            "work.react",
            json!({"comment_id": reply_id, "emoji": "👍"}),
        )
        .await;
    assert_eq!(
        w.call_as(
            w.principal("priya", ScopeSet::FULL),
            "work.edit_comment",
            json!({"comment_id": reply_id, "body": "x"})
        )
        .await,
        Err(ErrorKind::Conflict),
        "only the author edits"
    );
    let events = w.events.take();
    assert!(events.iter().any(|e| matches!(e, WorkEvent::CommentAdded { recipients, .. } if recipients.contains(&UserRef::parse("dev").unwrap()))));

    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 0);
    let threads = w
        .call("dev", "work.list_comments", json!({"target": "ENG-1"}))
        .await;
    assert_eq!(
        threads["threads"][0]["comments"][1]["reactions"][0]["emoji"],
        "👍"
    );
    let _ = w
        .call("dev", "work.resolve_thread", json!({"thread_id": thread}))
        .await;
    assert!(
        w.call("dev", "work.my_assignments", json!({})).await["assignments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn callers_need_a_handle_and_bad_input_changes_nothing() {
    let w = world().await;
    with_eng(&w).await;
    // A signed-in account without a Work handle.
    let stranger = w.principal("nobody", ScopeSet::FULL);
    assert_eq!(
        w.call_as(
            stranger,
            "work.create_issue",
            json!({"team": "ENG", "title": "x"})
        )
        .await,
        Err(ErrorKind::PermissionDenied)
    );
    assert_eq!(
        w.call_as(Principal::anonymous(), "work.my_issues", json!({}))
            .await,
        Err(ErrorKind::Unauthenticated)
    );
    for bad in [
        json!({"team": "NOPE", "title": "x"}),
        json!({"team": "ENG", "title": "x", "priority": 9}),
        json!({"team": "ENG", "title": "x", "due": "tomorrow"}),
        json!({"team": "ENG", "title": "x", "source_block": "not-a-block"}),
        json!({"team": "ENG", "title": "x", "state": "no-such-status"}),
    ] {
        assert!(
            w.call_as(
                w.principal("dev", ScopeSet::FULL),
                "work.create_issue",
                bad.clone()
            )
            .await
            .is_err(),
            "{bad}"
        );
    }
    let next = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "First real one"}),
        )
        .await;
    assert_eq!(
        next["issue"]["key"], "ENG-1",
        "refused input burned no number"
    );

    let team = std::fs::read_to_string(w.dir.path().join("teams/ENG/team.yaml")).unwrap();
    let renamed = team.replace("key: ENG", "key: OPS");
    assert_eq!(
        w.call_as(
            w.principal("mara", ScopeSet::FULL),
            "work.put_team",
            json!({"team": "ENG", "yaml": renamed})
        )
        .await,
        Err(ErrorKind::InvalidInput)
    );
    let edited = team.replace("name: Engineering", "name: Platform");
    let out = w
        .call(
            "mara",
            "work.put_team",
            json!({"team": "ENG", "yaml": edited}),
        )
        .await;
    assert_eq!(out["name"], "Platform");
}

#[tokio::test]
async fn archived_teams_leave_lists_and_take_no_new_issues() {
    let w = world().await;
    with_eng(&w).await;
    let _ = w
        .call(
            "mara",
            "work.create_team",
            json!({"key": "OPS", "name": "Operations"}),
        )
        .await;
    let kept = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "OPS", "title": "Rotate keys"}),
        )
        .await;
    assert_eq!(kept["issue"]["key"], "OPS-1");

    // Admin scope and a fresh sign-in, as every_operation_is_valid_and_unique checks.
    let out = w
        .call("mara", "work.archive_team", json!({"team": "OPS"}))
        .await;
    assert_eq!(out["archived"], true);
    let yaml = std::fs::read_to_string(w.dir.path().join("teams/OPS/team.yaml")).unwrap();
    assert!(yaml.contains("archived: true"), "{yaml}");

    // Gone from the default list, there when asked for.
    let keys = |v: &Value| -> Vec<String> {
        v["teams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["key"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        keys(&w.call("dev", "work.list_teams", json!({})).await),
        ["ENG"]
    );
    assert_eq!(
        keys(
            &w.call("dev", "work.list_teams", json!({"include_archived": true}))
                .await
        ),
        ["ENG", "OPS"]
    );

    // Its issue still reads; new work is refused with a conflict.
    assert_eq!(
        w.call("dev", "work.get_issue", json!({"key": "OPS-1"}))
            .await["title"],
        "Rotate keys"
    );
    assert_eq!(
        w.call("dev", "work.get_team", json!({"team": "OPS"})).await["archived"],
        true
    );
    let me = w.principal("dev", ScopeSet::FULL);
    assert_eq!(
        w.call_as(
            me.clone(),
            "work.create_issue",
            json!({"team": "OPS", "title": "New"})
        )
        .await,
        Err(ErrorKind::Conflict)
    );
    let eng = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Move me"}),
        )
        .await;
    assert_eq!(
        w.call_as(
            me,
            "work.move_issue",
            json!({"key": eng["issue"]["key"], "to": "OPS"})
        )
        .await,
        Err(ErrorKind::Conflict)
    );

    // It survives a restart, and restoring reopens it.
    let (w, _) = w.reopen().await;
    assert_eq!(
        keys(&w.call("dev", "work.list_teams", json!({})).await),
        ["ENG"]
    );
    let out = w
        .call("mara", "work.restore_team", json!({"team": "OPS"}))
        .await;
    assert_eq!(out["archived"], false);
    let next = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "OPS", "title": "Back"}),
        )
        .await;
    assert_eq!(
        next["issue"]["key"], "OPS-2",
        "numbers continue after a restore"
    );
    assert_eq!(
        w.call("mara", "work.restore_team", json!({"team": "OPS"}))
            .await["archived"],
        false,
        "restoring twice is harmless"
    );
}

#[tokio::test]
async fn a_corrupt_file_is_skipped_not_fatal() {
    let w = world().await;
    with_eng(&w).await;
    let _ = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Good"}),
        )
        .await;
    std::fs::write(
        w.dir.path().join("teams/ENG/issues/ENG-9.md"),
        "---\nnot: [valid\n---\n",
    )
    .unwrap();
    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 1);
    assert_eq!(
        w.call("dev", "work.get_issue", json!({"key": "ENG-1"}))
            .await["title"],
        "Good"
    );
}

#[tokio::test]
async fn forms_file_triage_issues_and_suggestions_wait_for_a_person() {
    let w = world().await;
    with_eng(&w).await;
    // A team with a label and a bug form, set the way an admin would.
    let path = w.dir.path().join("teams/ENG/team.yaml");
    let mut yaml = std::fs::read_to_string(&path).unwrap();
    yaml.push_str(
        "labels:\n- name: bug\n- name: gateway\nforms:\n- slug: bug\n  title: Report a bug\n  fields:\n  - key: summary\n    label: Summary\n    type: text\n    required: true\n    maps_to: title\n  - key: steps\n    label: Steps\n    type: long_text\n  labels: [bug]\n",
    );
    let _ = w
        .call(
            "mara",
            "work.put_team",
            json!({"team": "ENG", "yaml": yaml}),
        )
        .await;
    let forms = w
        .call("dev", "work.list_forms", json!({"team": "ENG"}))
        .await;
    assert_eq!(forms["forms"][0]["slug"], "bug");
    assert_eq!(forms["forms"][0]["fields"][1]["kind"], "long_text");

    // A member's submission lands in triage like any outside request.
    let issue = w
        .call(
            "dev",
            "work.submit_form",
            json!({"team": "ENG", "slug": "bug", "answers": {"summary": "Gateway times out", "steps": "Upload 2 GB"}}),
        )
        .await;
    assert_eq!(issue["title"], "Gateway times out");
    assert_eq!(issue["category"], "triage");
    assert_eq!(issue["labels"], json!(["bug"]));
    let key = issue["key"].as_str().unwrap().to_owned();
    let refused = w
        .call_as(
            w.principal("dev", ScopeSet::FULL),
            "work.submit_form",
            json!({"team": "ENG", "slug": "bug", "answers": {"steps": "no summary"}}),
        )
        .await;
    assert_eq!(refused, Err(ErrorKind::InvalidInput));

    // An assistant reads the brief and proposes; nothing changes yet.
    let brief = w
        .call("priya", "work.triage_brief", json!({"key": key}))
        .await;
    assert_eq!(brief["team"], "ENG");
    assert!(brief["suggestion"].is_null());
    let suggestion = w
        .call(
            "priya",
            "work.suggest_triage",
            json!({"key": key, "priority": 2, "labels": ["gateway"], "assignee": "priya", "reason": "Gateway timeouts are Priya's area", "model": "assistant"}),
        )
        .await;
    assert_eq!(suggestion["assignee"], "priya");
    let bad = w
        .call_as(
            w.principal("priya", ScopeSet::FULL),
            "work.suggest_triage",
            json!({"key": key, "assignee": "nobody", "reason": "x"}),
        )
        .await;
    assert_eq!(bad, Err(ErrorKind::InvalidInput));
    let still = w.call("dev", "work.get_issue", json!({"key": key})).await;
    assert_eq!(still["category"], "triage");
    assert!(still["assignee"].is_null());

    // A person accepts it; the issue leaves triage with the proposed fields.
    let accepted = w
        .call("mara", "work.accept_suggestion", json!({"key": key}))
        .await;
    assert_eq!(accepted["assignee"], "priya");
    assert_eq!(accepted["priority"], 2);
    assert_eq!(accepted["labels"], json!(["bug", "gateway"]));
    assert_ne!(accepted["category"], "triage");
    let brief_after = w
        .call_as(
            w.principal("dev", ScopeSet::FULL),
            "work.triage_brief",
            json!({"key": key}),
        )
        .await;
    assert_eq!(brief_after, Err(ErrorKind::Conflict));

    // A dismissed suggestion leaves the issue in triage, and survives a restart as gone.
    let other = w
        .call(
            "dev",
            "work.submit_form",
            json!({"team": "ENG", "slug": "bug", "answers": {"summary": "Login is slow"}}),
        )
        .await;
    let other_key = other["key"].as_str().unwrap().to_owned();
    let _ = w
        .call(
            "priya",
            "work.suggest_triage",
            json!({"key": other_key, "priority": 3, "reason": "Minor"}),
        )
        .await;
    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 0);
    let brief = w
        .call("dev", "work.triage_brief", json!({"key": other_key}))
        .await;
    assert_eq!(brief["suggestion"]["reason"], "Minor");
    let dismissed = w
        .call("dev", "work.dismiss_suggestion", json!({"key": other_key}))
        .await;
    assert_eq!(dismissed["category"], "triage");
    let brief = w
        .call("dev", "work.triage_brief", json!({"key": other_key}))
        .await;
    assert!(brief["suggestion"].is_null());
}

#[tokio::test]
async fn insights_count_finished_work_by_team_and_refuse_bad_ranges() {
    let w = world().await;
    with_eng(&w).await;
    for (title, state) in [
        ("Ship login", "done"),
        ("Fix upload", "done"),
        ("Plan Q1", "todo"),
    ] {
        let _ = w
            .call(
                "dev",
                "work.create_issue",
                json!({"team": "ENG", "title": title, "state": state, "assignee": "dev"}),
            )
            .await;
    }
    let answer = w
        .call(
            "mara",
            "work.insights",
            json!({"team": "ENG", "measure": "throughput", "pivot": "assignee", "from": "2026-10-26", "to": "2026-11-09", "interval": "week"}),
        )
        .await;
    assert_eq!(answer["overall"]["count"], 2);
    assert_eq!(answer["buckets"][0]["key"], "dev");
    assert_eq!(answer["buckets"][0]["series"].as_array().unwrap().len(), 2);

    let created = w
        .call(
            "mara",
            "work.insights",
            json!({"measure": "created", "from": "2026-11-01", "to": "2026-11-03"}),
        )
        .await;
    assert_eq!(created["overall"]["count"], 3);

    for bad in [
        json!({"from": "2026-11-03", "to": "2026-11-01"}),
        json!({"from": "2020-01-01", "to": "2026-11-01"}),
        json!({"measure": "speed", "from": "2026-11-01", "to": "2026-11-03"}),
        json!({"filter": "{", "from": "2026-11-01", "to": "2026-11-03"}),
        json!({"from": "Nov 1", "to": "2026-11-03"}),
    ] {
        let refused = w
            .call_as(
                w.principal("mara", ScopeSet::FULL),
                "work.insights",
                bad.clone(),
            )
            .await;
        assert_eq!(refused, Err(ErrorKind::InvalidInput), "{bad}");
    }
}

#[tokio::test]
async fn events_name_who_made_the_change() {
    let w = world().await;
    with_eng(&w).await;
    let _ = w.events.take_batches();
    let issue = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Self-assigned", "assignee": "dev"}),
        )
        .await;
    let batches = w.events.take_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].0, Some(UserRef::parse("dev").unwrap()));
    assert!(
        batches[0]
            .1
            .iter()
            .any(|e| matches!(e, WorkEvent::IssueCreated { .. }))
    );

    let key = issue["issue"]["key"].as_str().unwrap();
    let _ = w
        .call(
            "priya",
            "work.update_issue",
            json!({"key": key, "state": "in-progress"}),
        )
        .await;
    let batches = w.events.take_batches();
    assert_eq!(batches[0].0, Some(UserRef::parse("priya").unwrap()));
}

#[tokio::test]
async fn saved_views_are_shared_by_visibility_and_survive_a_restart() {
    let w = world().await;
    with_eng(&w).await;
    let filter = r#"{"or":[{"field":"priority","op":"is","values":["urgent"]},{"field":"assignee","op":"is","values":["@me"]}]}"#;
    let team_view = w
        .call(
            "dev",
            "work.create_view",
            json!({"name": "Urgent or mine", "team": "ENG", "visibility": "team",
                   "layout": "board", "group_by": "assignee", "filter": filter}),
        )
        .await;
    let team_id = team_view["id"].as_str().unwrap().to_owned();
    assert_eq!(team_view["layout"], "board");
    assert_eq!(team_view["owner"], "dev");
    let private = w
        .call("dev", "work.create_view", json!({"name": "Mine only"}))
        .await;
    let private_id = private["id"].as_str().unwrap().to_owned();

    // A team member sees the team view, not dev's private one.
    let seen = w.call("priya", "work.list_views", json!({})).await;
    let names: Vec<&str> = seen["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Urgent or mine"]);
    // Only the owner changes a view; a hidden view is "not found".
    let priya = w.principal("priya", ScopeSet::FULL);
    assert_eq!(
        w.call_as(
            priya.clone(),
            "work.put_view",
            json!({"view_id": team_id, "name": "Mine"})
        )
        .await,
        Err(ErrorKind::PermissionDenied)
    );
    assert_eq!(
        w.call_as(
            priya.clone(),
            "work.delete_view",
            json!({"view_id": private_id})
        )
        .await,
        Err(ErrorKind::NotFound)
    );
    // Bad input is refused.
    for bad in [
        json!({"name": ""}),
        json!({"name": "x", "visibility": "team"}),
        json!({"name": "x", "layout": "grid"}),
        json!({"name": "x", "filter": "{\"field\":\"nope\"}"}),
    ] {
        assert_eq!(
            w.call_as(priya.clone(), "work.create_view", bad.clone())
                .await,
            Err(ErrorKind::InvalidInput),
            "{bad}"
        );
    }

    let renamed = w
        .call(
            "dev",
            "work.put_view",
            json!({"view_id": team_id, "name": "Hot", "team": "ENG", "visibility": "team",
                   "layout": "board", "group_by": "assignee", "filter": filter}),
        )
        .await;
    assert_eq!(renamed["name"], "Hot");

    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 0);
    let views = w
        .call("dev", "work.list_views", json!({"team": "ENG"}))
        .await;
    let views = views["views"].as_array().unwrap();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0]["name"], "Hot");
    let back: Value = serde_json::from_str(views[0]["filter"].as_str().unwrap()).unwrap();
    assert_eq!(back, serde_json::from_str::<Value>(filter).unwrap());
    // The stored filter runs as a query.
    let _ = w
        .call(
            "dev",
            "work.query_issues",
            json!({"team": "ENG", "filter": views[0]["filter"], "group_by": "assignee"}),
        )
        .await;

    let _ = w
        .call("dev", "work.delete_view", json!({"view_id": private_id}))
        .await;
    let left = w.call("dev", "work.list_views", json!({})).await;
    assert_eq!(left["views"].as_array().unwrap().len(), 1);
    assert!(
        !w.dir
            .path()
            .join(format!("views/{private_id}.yaml"))
            .exists()
    );
}

#[tokio::test]
async fn team_settings_round_trip_as_json_and_numbers_never_go_back() {
    let w = world().await;
    with_eng(&w).await;
    let read = w
        .call("dev", "work.get_team_settings", json!({"team": "ENG"}))
        .await;
    let mut settings = read["settings"].clone();
    assert!(read["yaml"].as_str().unwrap().contains("key: ENG"));
    // Rename a status, add one, turn on one-week cycles, add an SLA rule
    // and map Git moments, as the settings screen does.
    let statuses = settings["workflow"]["statuses"].as_array_mut().unwrap();
    let todo = statuses.iter_mut().find(|s| s["key"] == "todo").unwrap();
    todo["name"] = json!("Up next");
    statuses.push(json!({"key": "qa", "name": "QA", "category": "started"}));
    settings["cycles"] = json!({"duration_weeks": 1, "start_day": "tuesday", "upcoming": 2});
    settings["sla"]["rules"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "Low", "when": {"field": "priority", "op": "is", "values": ["low"]}, "within_hours": 240}));
    settings["git"] = json!({"pr_opened": "qa", "pr_merged": "done"});
    // A stale copy: an issue is filed between the read and the save.
    let _ = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Filed meanwhile"}),
        )
        .await;
    let saved = w
        .call(
            "mara",
            "work.put_team",
            json!({"team": "ENG", "yaml": settings.to_string()}),
        )
        .await;
    assert!(
        saved["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "Up next")
    );
    assert_eq!(saved["cycles"], true);

    let (w, skipped) = w.reopen().await;
    assert_eq!(skipped, 0);
    let back = w
        .call("dev", "work.get_team_settings", json!({"team": "ENG"}))
        .await;
    assert_eq!(back["settings"]["git"]["pr_opened"], "qa");
    assert_eq!(back["settings"]["cycles"]["duration_weeks"], 1);
    assert_eq!(
        back["settings"]["last_number"], 1,
        "the stale copy did not reset numbering"
    );
    let next = w
        .call(
            "dev",
            "work.create_issue",
            json!({"team": "ENG", "title": "Next"}),
        )
        .await;
    assert_eq!(next["issue"]["key"], "ENG-2");

    // A Git moment naming a missing status, or a changed team ID, is refused.
    let mara = w.principal("mara", ScopeSet::FULL);
    let mut bad = back["settings"].clone();
    bad["git"] = json!({"pr_merged": "shipped"});
    assert_eq!(
        w.call_as(
            mara.clone(),
            "work.put_team",
            json!({"team": "ENG", "yaml": bad.to_string()})
        )
        .await,
        Err(ErrorKind::InvalidInput)
    );
    let mut moved = back["settings"].clone();
    moved["id"] = json!("01JBV0000000000000000000ZZ");
    assert!(
        w.call_as(
            mara,
            "work.put_team",
            json!({"team": "ENG", "yaml": moved.to_string()})
        )
        .await
        .is_err()
    );
}
