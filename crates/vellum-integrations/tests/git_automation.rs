// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Git automation end to end, against an in-memory issue core.

// Test helpers outside #[test] functions; clippy.toml allows these in tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use jiff::Timestamp;
use vellum_integrations::git::automation::{IssueLinks, Mapping};
use vellum_integrations::git::event::{
    GitEvent, PrRef, Provider, PullRequestAction,
};
use vellum_integrations::git::service::{
    LinkStore, Repository, Settings, apply,
};
use vellum_integrations::git::{github, refs::RefKind};
use vellum_integrations::model::{
    IssueDraft, IssueKey, IssueSnapshot, StateCategory, WorkCommands,
    WorkError, WorkflowState,
};

fn states() -> Vec<WorkflowState> {
    [
        ("todo", "Todo", StateCategory::Unstarted),
        ("prog", "In Progress", StateCategory::Started),
        ("rev", "In Review", StateCategory::Started),
        ("rdy", "Ready to merge", StateCategory::Started),
        ("done", "Done", StateCategory::Completed),
    ]
    .into_iter()
    .map(|(id, name, category)| WorkflowState {
        id: id.to_owned(),
        name: name.to_owned(),
        category,
    })
    .collect()
}

#[derive(Default)]
struct Core {
    issues: Mutex<BTreeMap<IssueKey, IssueSnapshot>>,
    links: Mutex<BTreeMap<IssueKey, IssueLinks>>,
    history: Mutex<Vec<String>>,
    /// The mapping the team set in its own settings, if any.
    own: Option<Mapping>,
}

fn key(text: &str) -> IssueKey {
    IssueKey::parse(text).unwrap()
}

impl Core {
    fn with_issues(keys: &[&str]) -> Self {
        let core = Self::default();
        let todo = states().into_iter().next().unwrap();
        for k in keys {
            let _ = core.issues.lock().unwrap().insert(
                key(k),
                IssueSnapshot {
                    key: key(k),
                    title: format!("Issue {k}"),
                    state: todo.clone(),
                    assignee: None,
                    created_at: Timestamp::UNIX_EPOCH,
                },
            );
        }
        core
    }

    fn state(&self, k: &str) -> String {
        self.issues.lock().unwrap()[&key(k)].state.id.clone()
    }
}

impl WorkCommands for Core {
    async fn issue(
        &self,
        key: &IssueKey,
    ) -> Result<Option<IssueSnapshot>, WorkError> {
        Ok(self.issues.lock().unwrap().get(key).cloned())
    }

    async fn states(
        &self,
        _team: &str,
    ) -> Result<Vec<WorkflowState>, WorkError> {
        Ok(states())
    }

    async fn set_state(
        &self,
        key: &IssueKey,
        state_id: &str,
        reason: &str,
    ) -> Result<(), WorkError> {
        let state = states().into_iter().find(|s| s.id == state_id).unwrap();
        self.issues.lock().unwrap().get_mut(key).unwrap().state = state;
        self.history
            .lock()
            .unwrap()
            .push(format!("{key} -> {state_id} ({reason})"));
        Ok(())
    }

    async fn create_issue(
        &self,
        _draft: IssueDraft,
    ) -> Result<IssueKey, WorkError> {
        Err(WorkError::Refused("not used".to_owned()))
    }

    async fn add_comment(
        &self,
        _key: &IssueKey,
        _author: Option<&str>,
        _body: &str,
    ) -> Result<(), WorkError> {
        Ok(())
    }

    async fn git_mapping(
        &self,
        _team: &str,
    ) -> Result<Option<Mapping>, WorkError> {
        Ok(self.own.clone())
    }
}

impl LinkStore for Core {
    async fn load(&self, key: &IssueKey) -> Result<IssueLinks, WorkError> {
        Ok(self
            .links
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default())
    }

    async fn save(
        &self,
        key: &IssueKey,
        links: &IssueLinks,
    ) -> Result<(), WorkError> {
        let _ = self
            .links
            .lock()
            .unwrap()
            .insert(key.clone(), links.clone());
        Ok(())
    }

    async fn issues_for_pr(
        &self,
        pr: &PrRef,
    ) -> Result<BTreeSet<IssueKey>, WorkError> {
        Ok(self
            .links
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| l.prs.iter().any(|p| &p.pr == pr))
            .map(|(k, _)| k.clone())
            .collect())
    }
}

fn settings() -> Settings {
    Settings {
        repositories: vec![Repository {
            provider: Provider::GitHub,
            repo: "northstar/api".to_owned(),
            teams: std::iter::once("ENG".to_owned()).collect(),
        }],
        mappings: std::iter::once((
            "ENG".to_owned(),
            Mapping::default_for(&states()),
        ))
        .collect(),
    }
}

fn pr(
    number: u64,
    action: PullRequestAction,
    title: &str,
    body: &str,
) -> GitEvent {
    GitEvent::PullRequest {
        pr: PrRef {
            provider: Provider::GitHub,
            repo: "northstar/api".to_owned(),
            number,
        },
        action,
        title: title.to_owned(),
        body: body.to_owned(),
        branch: format!("feature-{number}"),
        url: format!("https://github.com/northstar/api/pull/{number}"),
        draft: false,
    }
}

#[tokio::test]
async fn issue_completes_only_when_every_closing_pr_merges() {
    let core = Core::with_issues(&["ENG-1", "ENG-2", "ENG-3"]);
    let settings = settings();
    let run = |event: GitEvent| {
        let core = &core;
        let settings = &settings;
        async move {
            let _ = apply(core, core, settings, &event).await.unwrap();
        }
    };

    // A branch named for ENG-1 starts it.
    let push = GitEvent::Push {
        provider: Provider::GitHub,
        repo: "northstar/api".to_owned(),
        branch: "ada/eng-1-fix-login".to_owned(),
        default_branch: false,
        commits: Vec::new(),
    };
    run(push).await;
    assert_eq!(core.state("ENG-1"), "prog");

    // PR 10 closes ENG-1 (title) and ENG-2 (magic word), links ENG-3,
    // and mentions OPS-1, which this repository may not touch.
    run(pr(
        10,
        PullRequestAction::Opened,
        "ENG-1 Fix login",
        "Fixes ENG-2. Part of ENG-3. Closes OPS-1",
    ))
    .await;
    assert_eq!(
        (
            core.state("ENG-1"),
            core.state("ENG-2"),
            core.state("ENG-3")
        ),
        ("rev".into(), "rev".into(), "todo".into())
    );
    let eng3 = core.links.lock().unwrap()[&key("ENG-3")].clone();
    assert_eq!(eng3.prs.first().map(|l| l.kind), Some(RefKind::Linking));

    // A second PR also closes ENG-1.
    run(pr(
        11,
        PullRequestAction::Opened,
        "Follow-up",
        "Resolves ENG-1",
    ))
    .await;
    run(pr(
        10,
        PullRequestAction::Approved,
        "ENG-1 Fix login",
        "Fixes ENG-2. Part of ENG-3",
    ))
    .await;
    assert_eq!(core.state("ENG-1"), "rev", "PR 11 is still in review");
    assert_eq!(core.state("ENG-2"), "rdy");

    run(pr(
        10,
        PullRequestAction::Merged,
        "ENG-1 Fix login",
        "Fixes ENG-2. Part of ENG-3",
    ))
    .await;
    assert_eq!(core.state("ENG-1"), "rev", "all closing PRs must merge");
    assert_eq!(core.state("ENG-2"), "done");
    assert_eq!(core.state("ENG-3"), "todo", "linking PRs never move issues");

    // PR 11 drops its reference; ENG-1's only closing PR has merged.
    run(pr(
        11,
        PullRequestAction::Edited,
        "Follow-up",
        "No longer related",
    ))
    .await;
    assert_eq!(core.state("ENG-1"), "done");
    assert!(core.history.lock().unwrap().iter().any(|h| h.starts_with(
        "ENG-1 -> done (GitHub northstar/api#11 no longer references it)"
    )));
}

#[tokio::test]
async fn closing_unmerged_returns_the_issue_and_webhooks_parse() {
    let core = Core::with_issues(&["ENG-7"]);
    let settings = settings();
    let body = serde_json::json!({
        "action": "opened",
        "repository": {"full_name": "northstar/api", "default_branch": "main"},
        "pull_request": {"number": 3, "title": "Speed up search", "body": "Closes ENG-7",
                         "html_url": "https://github.com/northstar/api/pull/3", "draft": false,
                         "merged": false, "head": {"ref": "speed"}}
    })
    .to_string();
    let signature =
        format!("sha256={}", hmac_hex(b"hook-secret", body.as_bytes()));
    github::verify(b"hook-secret", Some(&signature), body.as_bytes()).unwrap();
    let event = github::parse("pull_request", body.as_bytes())
        .unwrap()
        .unwrap();
    let applied = apply(&core, &core, &settings, &event).await.unwrap();
    assert_eq!(
        applied.first().and_then(|a| a.moved_to.as_deref()),
        Some("rev")
    );

    let closed = body.replace("\"opened\"", "\"closed\"");
    let event = github::parse("pull_request", closed.as_bytes())
        .unwrap()
        .unwrap();
    let _ = apply(&core, &core, &settings, &event).await.unwrap();
    assert_eq!(core.state("ENG-7"), "todo");

    // A closing commit pushed straight to the default branch completes it.
    let push = GitEvent::Push {
        provider: Provider::GitHub,
        repo: "northstar/api".to_owned(),
        branch: "main".to_owned(),
        default_branch: true,
        commits: vec![vellum_integrations::git::event::Commit {
            sha: "abc".to_owned(),
            message: "Hotfix\n\nFixes ENG-7".to_owned(),
            url: None,
        }],
    };
    let _ = apply(&core, &core, &settings, &push).await.unwrap();
    assert_eq!(core.state("ENG-7"), "done");
}

fn hmac_hex(key: &[u8], body: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

#[tokio::test]
async fn the_team_settings_mapping_wins_over_the_integration_file() {
    let mut core = Core::with_issues(&["ENG-9"]);
    // The team sends opened pull requests straight to Ready to merge, and
    // leaves merges alone.
    core.own = Some(Mapping {
        pr_opened: Some("rdy".to_owned()),
        ..Mapping::default()
    });
    let settings = settings();
    let opened = pr(9, PullRequestAction::Opened, "Fix", "Closes ENG-9");
    let applied = apply(&core, &core, &settings, &opened).await.unwrap();
    assert_eq!(applied[0].moved_to.as_deref(), Some("rdy"));
    let merged = pr(9, PullRequestAction::Merged, "Fix", "Closes ENG-9");
    let _ = apply(&core, &core, &settings, &merged).await.unwrap();
    assert_eq!(
        core.state("ENG-9"),
        "rdy",
        "an unmapped moment does not move the issue"
    );
}
