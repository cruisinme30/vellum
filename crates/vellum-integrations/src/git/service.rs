// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Applying a Git event: link, decide, transition.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use tracing::Instrument as _;

use super::automation::{IssueLinks, Mapping, PrLink, PrStatus, decide};
use super::event::{Commit, GitEvent, PrRef, Provider};
use super::refs::{Collector, Location, RefKind, Reference};
use crate::model::{IssueKey, WorkCommands, WorkError};

/// Storage for the links this service owns.
pub trait LinkStore: Send + Sync {
    /// The links of an issue (empty when none).
    fn load(
        &self,
        key: &IssueKey,
    ) -> impl Future<Output = Result<IssueLinks, WorkError>> + Send;
    /// Saves the links of an issue.
    fn save(
        &self,
        key: &IssueKey,
        links: &IssueLinks,
    ) -> impl Future<Output = Result<(), WorkError>> + Send;
    /// The issues a pull request is linked to now.
    fn issues_for_pr(
        &self,
        pr: &PrRef,
    ) -> impl Future<Output = Result<BTreeSet<IssueKey>, WorkError>> + Send;
}

/// One connected repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The provider.
    pub provider: Provider,
    /// `owner/repo` or the GitLab project path.
    pub repo: String,
    /// Team keys this repository may act on.
    pub teams: BTreeSet<String>,
}

/// The integration's settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// Connected repositories.
    pub repositories: Vec<Repository>,
    /// Each team's mapping, by team key.
    pub mappings: BTreeMap<String, Mapping>,
}

impl Settings {
    fn teams_for(&self, provider: Provider, repo: &str) -> BTreeSet<String> {
        self.repositories
            .iter()
            .filter(|r| {
                r.provider == provider && r.repo.eq_ignore_ascii_case(repo)
            })
            .flat_map(|r| r.teams.iter().cloned())
            .collect()
    }
}

/// What happened to one issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The issue.
    pub key: IssueKey,
    /// The state it moved to, if it moved.
    pub moved_to: Option<String>,
}

/// Applies one event to every issue it references, and to issues a pull
/// request no longer references.
///
/// # Errors
///
/// [`WorkError`] from the issue core or the link store; the job is retried
/// and the steps are idempotent.
pub async fn apply<W: WorkCommands, L: LinkStore>(
    work: &W,
    store: &L,
    settings: &Settings,
    event: &GitEvent,
) -> Result<Vec<Applied>, WorkError> {
    let (provider, repo) = event.repo();
    let span = tracing::info_span!(
        "integrations.git.apply",
        provider = ?provider,
        repo,
        references = tracing::field::Empty
    );
    async {
        let teams = settings.teams_for(provider, repo);
        let references = event.collector().finish(&teams);
        let _ = tracing::Span::current().record("references", references.len());
        let mut targets: BTreeMap<IssueKey, Option<RefKind>> = references
            .iter()
            .map(|r: &Reference| (r.key.clone(), Some(r.kind)))
            .collect();
        if let GitEvent::PullRequest { pr, .. } = event {
            for stale in store.issues_for_pr(pr).await? {
                let _ = targets.entry(stale).or_insert(None);
            }
        }
        let mut applied = Vec::new();
        for (key, kind) in targets {
            if let Some(result) =
                apply_one(work, store, settings, event, &key, kind).await?
            {
                applied.push(result);
            }
        }
        tracing::info!(
            monotonic_counter.vellum.integrations.git_events = 1_u64,
            provider = ?provider,
            issues = applied.len(),
        );
        Ok(applied)
    }
    .instrument(span)
    .await
}

async fn apply_one<W: WorkCommands, L: LinkStore>(
    work: &W,
    store: &L,
    settings: &Settings,
    event: &GitEvent,
    key: &IssueKey,
    kind: Option<RefKind>,
) -> Result<Option<Applied>, WorkError> {
    let Some(issue) = work.issue(key).await? else {
        tracing::debug!(%key, "referenced issue does not exist");
        return Ok(None);
    };
    // A moved issue resolves to its new key; its team must still be one
    // the repository may act on.
    if issue.key.team != key.team && kind.is_some() {
        let teams = settings.teams_for(event.repo().0, event.repo().1);
        if !teams.contains(&issue.key.team) {
            return Ok(None);
        }
    }
    let mut links = store.load(&issue.key).await?;
    let reason = match (event, kind) {
        (GitEvent::PullRequest { pr, .. }, None) => {
            if !links.unlink(pr) {
                return Ok(None);
            }
            format!(
                "{} {}#{} no longer references it",
                provider_name(pr.provider),
                pr.repo,
                pr.number
            )
        }
        (
            GitEvent::PullRequest {
                pr,
                action,
                title,
                url,
                draft,
                ..
            },
            Some(kind),
        ) => {
            let status = PrStatus::after(links.status_of(pr), *action, *draft);
            links.upsert(PrLink {
                pr: pr.clone(),
                kind,
                status,
                title: title.clone(),
                url: url.clone(),
            });
            format!(
                "{} {}#{} {:?}",
                provider_name(pr.provider),
                pr.repo,
                pr.number,
                status
            )
        }
        (
            GitEvent::Push {
                repo,
                branch,
                default_branch,
                commits,
                ..
            },
            Some(_),
        ) => {
            record_push(
                &mut links,
                key,
                repo,
                branch,
                *default_branch,
                commits,
            );
            format!("branch {branch} pushed to {repo}")
        }
        (GitEvent::Push { .. }, None) => return Ok(None),
    };
    store.save(&issue.key, &links).await?;
    let own = work.git_mapping(&issue.key.team).await?;
    let mapping = own
        .as_ref()
        .or_else(|| settings.mappings.get(&issue.key.team));
    let moved_to = match mapping {
        Some(mapping) => match decide(&issue, links.stage(), mapping) {
            Some(target) => {
                work.set_state(&issue.key, target, &reason).await?;
                Some(target.to_owned())
            }
            None => None,
        },
        None => None,
    };
    Ok(Some(Applied {
        key: issue.key,
        moved_to,
    }))
}

/// Records a push's branch and commits on one issue's links.
fn record_push(
    links: &mut IssueLinks,
    key: &IssueKey,
    repo: &str,
    branch: &str,
    default_branch: bool,
    commits: &[Commit],
) {
    let only_this: BTreeSet<String> =
        std::iter::once(key.team.clone()).collect();
    let kind_in = |location: Location, text: &str| {
        let mut collector = Collector::new();
        let _ = collector.add(location, text);
        collector
            .finish(&only_this)
            .into_iter()
            .find(|r| &r.key == key)
            .map(|r| r.kind)
    };
    if !default_branch && kind_in(Location::Branch, branch).is_some() {
        let _ = links.branches.insert(format!("{repo}:{branch}"));
    }
    for commit in commits {
        let set = match (
            kind_in(Location::Commit, &commit.message),
            default_branch,
        ) {
            (Some(RefKind::Closing), true) => &mut links.closing_commits,
            (Some(RefKind::Closing), false) => &mut links.commits,
            (Some(RefKind::Linking), _) => &mut links.linked_commits,
            (None, _) => continue,
        };
        let _ = set.insert(commit.sha.clone());
    }
}

const fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::GitHub => "GitHub",
        Provider::GitLab => "GitLab",
    }
}
