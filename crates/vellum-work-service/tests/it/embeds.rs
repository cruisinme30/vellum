// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Live `::: issues` views: every block of a page runs for the reader and
//! reports its own result.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test fixtures fail loudly"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use time::macros::datetime;
use vellum_contracts::{CredentialKind, Operation, Principal, RequestContext, ScopeSet, Service};
use vellum_format::parse;
use vellum_kernel::{RequestId, Timestamp, UserId};
use vellum_render::figure::{FigureConfig, Figures};
use vellum_render::resolve::{IssueLayout, IssuesAt};
use vellum_render::{RenderOptions, Renderer};
use vellum_store::LocalStore;
use vellum_work::UserRef;
use vellum_work_service::{EventLog, FixedClock, StaticDirectory, WorkService};

async fn call(ops: &[Operation], who: UserId, op: &str, input: Value) -> Value {
    let op = ops.iter().find(|o| o.id() == op).unwrap();
    let ctx = RequestContext {
        request_id: RequestId::generate(),
        principal: Principal::user(
            who,
            CredentialKind::Session,
            ScopeSet::FULL,
            Timestamp::now(),
        ),
        deadline: Instant::now() + Duration::from_secs(5),
        traceparent: None,
    };
    op.call(ctx, input).await.unwrap()
}

#[tokio::test]
async fn each_issues_block_shows_its_own_view_or_its_own_error() {
    let dir = tempfile::tempdir().unwrap();
    let mara = UserId::generate();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let directory = StaticDirectory::new().with(mara, UserRef::parse("mara").unwrap());
    let (svc, _) = WorkService::open(
        store,
        Arc::new(directory),
        Arc::new(EventLog::new()),
        Arc::new(FixedClock(datetime!(2026-11-02 09:00 UTC))),
    )
    .await
    .unwrap();
    let svc = Arc::new(svc);
    let ops = svc.clone().operations();
    let _ = call(
        &ops,
        mara,
        "work.create_team",
        json!({"key": "ENG", "name": "Engineering", "lead": "mara"}),
    )
    .await;
    for title in ["Ship login", "Fix upload", "Plan Q1"] {
        let _ = call(&ops, mara, "work.create_issue", json!({"team": "ENG", "title": title, "state": "todo", "assignee": "mara", "priority": 2})).await;
    }

    let page = "\
# Launch

::: issues ^blk-aaaaaaaa
filter:
  field: team
  op: is
  values: [ENG]
group_by: status
layout: board
limit: 2
:::

::: issues
filter: [not, a, filter]
:::

::: issues
view: 01JC0000000000000000000V01
:::
";
    let found = svc
        .issue_embeds(page, &UserRef::parse("mara").unwrap())
        .await;
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].block_id.as_deref(), Some("blk-aaaaaaaa"));
    let view = found[0].view.as_ref().unwrap();
    assert!(view.board);
    assert_eq!(view.more, 1);
    let todo = view.groups.iter().find(|g| g.title == "Todo").unwrap();
    assert_eq!(todo.issues.len(), 2);
    assert_eq!(todo.issues[0].href, format!("/i/{}", todo.issues[0].key));
    assert_eq!(todo.issues[0].state, "Todo");
    assert_eq!(todo.issues[0].assignee.as_deref(), Some("mara"));
    assert!(found[1].view.is_err());
    assert!(found[2].view.as_ref().unwrap_err().contains("not found"));

    // Render draws the same page with real rows through the lookup.
    let live = svc
        .live_issues(page, &UserRef::parse("mara").unwrap())
        .await;
    let renderer = Renderer::new(Figures::new(FigureConfig::default()).unwrap());
    let html = renderer
        .render_offline(&parse(page).document, &live, &RenderOptions::default())
        .html;
    assert!(html.contains("Ship login"), "{html}");
    assert!(html.contains("/i/ENG-"), "{html}");
    let by_id = live
        .view(IssuesAt {
            index: 9,
            block_id: Some("blk-aaaaaaaa"),
        })
        .unwrap()
        .unwrap();
    assert_eq!(by_id.layout, IssueLayout::Board);
    assert_eq!(
        by_id.groups.iter().map(|g| g.issues.len()).sum::<usize>(),
        2
    );
    let first = by_id.groups.iter().find(|g| !g.issues.is_empty()).unwrap();
    assert_eq!(first.issues[0].priority.as_deref(), Some("High"));
    assert!(
        live.view(IssuesAt {
            index: 1,
            block_id: None
        })
        .unwrap()
        .is_err()
    );
    assert!(
        live.view(IssuesAt {
            index: 7,
            block_id: None
        })
        .is_none()
    );

    assert!(
        svc.issue_embeds("no blocks here", &UserRef::parse("mara").unwrap())
            .await
            .is_empty()
    );
}
