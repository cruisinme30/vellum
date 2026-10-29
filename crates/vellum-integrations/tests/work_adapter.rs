// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The integrations driving the real issue tracker engine.

// Test helpers outside #[test] functions; clippy.toml allows these in tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use vellum_integrations::git::automation::{IssueLinks, Mapping};
use vellum_integrations::git::event::{GitEvent, PrRef, Provider, PullRequestAction};
use vellum_integrations::git::service::{LinkStore, Repository, Settings, apply};
use vellum_integrations::model::{
    IssueDraft, IssueKey, Source, StateCategory, WorkCommands, WorkError,
};
use vellum_integrations::work::{Commit, CommitSink, GraphWork};
use vellum_kernel::TeamId;
use vellum_work::{Comments, Team, TeamKey, UserRef, WorkGraph};

#[derive(Default)]
struct Recorder(Mutex<Vec<Commit>>);

struct Sink(Arc<Recorder>);

impl CommitSink for Sink {
    async fn commit(&self, commit: Commit) -> Result<(), WorkError> {
        self.0.0.lock().unwrap().push(commit);
        Ok(())
    }
}

fn fixed() -> Timestamp {
    "2026-10-29T09:00:00Z".parse().unwrap()
}

fn setup() -> (GraphWork<Sink>, Arc<Mutex<WorkGraph>>, Arc<Recorder>) {
    let mut graph = WorkGraph::new();
    let team = Team::new(
        TeamId::generate(),
        TeamKey::parse("ENG").unwrap(),
        "Engineering",
    )
    .unwrap();
    graph.insert_team(team).unwrap();
    let graph = Arc::new(Mutex::new(graph));
    let recorder = Arc::new(Recorder::default());
    let work = GraphWork::new(
        Arc::clone(&graph),
        Arc::new(Mutex::new(Comments::new())),
        Sink(Arc::clone(&recorder)),
        UserRef::parse("vellum-bot").unwrap(),
    )
    .with_clock(fixed);
    (work, graph, recorder)
}

fn key(text: &str) -> IssueKey {
    IssueKey::parse(text).unwrap()
}

#[tokio::test]
async fn intake_lands_in_triage_once() {
    let (work, _graph, recorder) = setup();
    let states = work.states("eng").await.unwrap();
    assert!(states.iter().any(|s| s.id == "in-review"));

    let mut draft = IssueDraft::new(
        "ENG",
        "TypeError in checkout",
        Source::Sentry {
            issue_id: "4711".to_owned(),
        },
    );
    draft.description = "Seen 42 times".to_owned();
    let first = work.create_issue(draft.clone()).await.unwrap();
    assert_eq!(first.to_string(), "ENG-1");
    let issue = work.issue(&first).await.unwrap().unwrap();
    assert_eq!(issue.state.category, StateCategory::Triage);
    assert_eq!(issue.created_at, fixed());

    // Sentry retries the alert: the same report is not filed twice.
    let again = work.create_issue(draft).await.unwrap();
    assert_eq!(again, first);
    assert_eq!(recorder.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn state_changes_and_comments_are_committed_with_their_reason() {
    let (work, _graph, recorder) = setup();
    let issue = work
        .create_issue(IssueDraft::new("ENG", "Speed up search", Source::App))
        .await
        .unwrap();
    work.set_state(&issue, "in-review", "GitHub northstar/api#3 opened")
        .await
        .unwrap();
    assert_eq!(
        work.issue(&issue).await.unwrap().unwrap().state.name,
        "In Review"
    );
    work.add_comment(&issue, None, "Linked northstar/api#3")
        .await
        .unwrap();

    let commits = recorder.0.lock().unwrap().clone();
    assert_eq!(
        commits[1].reason.as_deref(),
        Some("GitHub northstar/api#3 opened")
    );
    assert!(commits[2].comments.is_some());

    assert!(matches!(
        work.set_state(&key("ENG-99"), "done", "x").await,
        Err(WorkError::NotFound(_))
    ));
    assert!(matches!(
        work.set_state(&issue, "no-such-state", "x").await,
        Err(WorkError::Refused(_))
    ));
}

#[derive(Default)]
struct Links(Mutex<BTreeMap<IssueKey, IssueLinks>>);

impl LinkStore for Links {
    async fn load(&self, key: &IssueKey) -> Result<IssueLinks, WorkError> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }

    async fn save(&self, key: &IssueKey, links: &IssueLinks) -> Result<(), WorkError> {
        let _ = self.0.lock().unwrap().insert(key.clone(), links.clone());
        Ok(())
    }

    async fn issues_for_pr(&self, pr: &PrRef) -> Result<BTreeSet<IssueKey>, WorkError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| l.prs.iter().any(|p| &p.pr == pr))
            .map(|(k, _)| k.clone())
            .collect())
    }
}

#[tokio::test]
async fn git_automation_moves_engine_issues() {
    let (work, _graph, _recorder) = setup();
    let issue = work
        .create_issue(IssueDraft::new("ENG", "Fix login", Source::App))
        .await
        .unwrap();
    let states = work.states("ENG").await.unwrap();
    let settings = Settings {
        repositories: vec![Repository {
            provider: Provider::GitHub,
            repo: "northstar/api".to_owned(),
            teams: std::iter::once("ENG".to_owned()).collect(),
        }],
        mappings: std::iter::once(("ENG".to_owned(), Mapping::default_for(&states))).collect(),
    };
    let links = Links::default();
    let pr = |action| GitEvent::PullRequest {
        pr: PrRef {
            provider: Provider::GitHub,
            repo: "northstar/api".to_owned(),
            number: 12,
        },
        action,
        title: "Fix login".to_owned(),
        body: format!("Fixes {issue}"),
        branch: "fix-login".to_owned(),
        url: "https://github.com/northstar/api/pull/12".to_owned(),
        draft: false,
    };

    let _ = apply(&work, &links, &settings, &pr(PullRequestAction::Opened))
        .await
        .unwrap();
    assert_eq!(
        work.issue(&issue).await.unwrap().unwrap().state.id,
        "in-review"
    );
    let _ = apply(&work, &links, &settings, &pr(PullRequestAction::Merged))
        .await
        .unwrap();
    assert_eq!(
        work.issue(&issue).await.unwrap().unwrap().state.category,
        StateCategory::Completed
    );
}
