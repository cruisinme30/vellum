// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Commands run from inside the server, as integrations run them: under
//! the service lock, written to files, announced, and rolled back on
//! failure.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use std::sync::Arc;

use time::macros::datetime;
use vellum_kernel::{Error, ErrorKind};
use vellum_store::LocalStore;
use vellum_work::{Changes, IssueId, NewIssue, Team, TeamKey, UserRef, WorkEvent};
use vellum_work_service::{EventLog, FixedClock, StaticDirectory, WorkService};

async fn open(dir: &std::path::Path) -> (WorkService, Arc<EventLog>) {
    let store = Arc::new(LocalStore::open(dir).await.unwrap());
    let events = Arc::new(EventLog::new());
    let (svc, _) = WorkService::open(
        store,
        Arc::new(StaticDirectory::new()),
        events.clone(),
        Arc::new(FixedClock(datetime!(2026-11-02 09:00 UTC))),
    )
    .await
    .unwrap();
    (svc, events)
}

#[tokio::test]
async fn a_bot_command_is_written_announced_or_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, events) = open(dir.path()).await;
    let bot = UserRef::parse("github-bot").unwrap();
    let eng = TeamKey::parse("ENG").unwrap();

    let team = Team::new(
        "01JC00000000000000000000E1".parse().unwrap(),
        eng.clone(),
        "Engineering",
    )
    .unwrap();
    svc.transact(bot.clone(), |graph, _, _| {
        graph.insert_team(team.clone())?;
        let mut changes = Changes::default();
        let _ = changes.teams.insert(TeamKey::parse("ENG")?);
        Ok((changes, None, ()))
    })
    .await
    .unwrap();

    let id = IssueId::generate();
    let key = svc
        .transact(bot.clone(), |graph, _, ctx| {
            let changes =
                graph.create_issue(NewIssue::new(id, eng.clone(), "Build fails on main"), ctx)?;
            let key = graph
                .issue(&id)
                .map(|i| i.key().to_string())
                .unwrap_or_default();
            Ok((changes, None, key))
        })
        .await
        .unwrap();
    assert_eq!(key, "ENG-1");
    let created = events.take();
    assert!(
        created
            .iter()
            .any(|e| matches!(e, WorkEvent::IssueCreated { .. }))
    );

    // A command that changes memory and then fails leaves nothing behind.
    let other = IssueId::generate();
    let failed = svc
        .transact(bot.clone(), |graph, _, ctx| {
            let _ = graph.create_issue(NewIssue::new(other, eng.clone(), "Half done"), ctx)?;
            Err::<(Changes, _, ()), _>(Error::new(ErrorKind::InvalidInput, "stop".to_owned()))
        })
        .await;
    assert_eq!(failed.map_err(|e| e.kind()), Err(ErrorKind::InvalidInput));
    assert!(svc.read(|graph, _| graph.issue(&other).is_none()).await);
    assert!(events.take().is_empty());

    // The created issue was written to disk and comes back on restart.
    drop(svc);
    let (again, _) = open(dir.path()).await;
    let title = again
        .read(|graph, _| graph.resolve("ENG-1").map(|i| i.title.clone()))
        .await;
    assert_eq!(title.as_deref(), Some("Build fails on main"));
}
