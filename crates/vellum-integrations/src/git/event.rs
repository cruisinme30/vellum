// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Provider-neutral Git events.

use serde::{Deserialize, Serialize};

use super::refs::{Collector, Location};

/// Which provider sent the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// GitHub or GitHub Enterprise.
    GitHub,
    /// GitLab, hosted or self-managed.
    GitLab,
}

/// Identifies a pull or merge request across providers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrRef {
    /// The provider.
    pub provider: Provider,
    /// `owner/repo` or the GitLab project path.
    pub repo: String,
    /// The pull request number (GitLab `iid`).
    pub number: u64,
}

/// One pushed commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// The commit SHA.
    pub sha: String,
    /// The full message.
    pub message: String,
    /// Link to the commit.
    pub url: Option<String>,
}

/// What happened to a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAction {
    /// Opened or reopened as ready for review.
    Opened,
    /// Opened, or converted, as a draft.
    Draft,
    /// Marked ready for review.
    ReadyForReview,
    /// Title, description or commits changed.
    Edited,
    /// A reviewer approved.
    Approved,
    /// A reviewer withdrew approval or requested changes.
    ChangesRequested,
    /// Merged.
    Merged,
    /// Closed without merging.
    ClosedUnmerged,
}

/// A normalised event from a Git provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GitEvent {
    /// Commits were pushed to a branch.
    Push {
        /// The provider.
        provider: Provider,
        /// The repository.
        repo: String,
        /// The branch, without `refs/heads/`.
        branch: String,
        /// Whether it is the repository's default branch.
        default_branch: bool,
        /// The pushed commits, oldest first.
        commits: Vec<Commit>,
    },
    /// A pull or merge request changed.
    PullRequest {
        /// Which request.
        pr: PrRef,
        /// What happened.
        action: PullRequestAction,
        /// Its title.
        title: String,
        /// Its description.
        body: String,
        /// Its source branch.
        branch: String,
        /// Link to it.
        url: String,
        /// Whether it is a draft now.
        draft: bool,
    },
}

impl GitEvent {
    /// The texts to scan for issue references, by location.
    #[must_use]
    pub fn collector(&self) -> Collector {
        let mut collector = Collector::new();
        match self {
            Self::Push {
                branch, commits, ..
            } => {
                let _ = collector.add(Location::Branch, branch);
                for commit in commits {
                    let _ = collector.add(Location::Commit, &commit.message);
                }
            }
            Self::PullRequest {
                title,
                body,
                branch,
                ..
            } => {
                let _ = collector
                    .add(Location::Branch, branch)
                    .add(Location::Title, title)
                    .add(Location::Body, body);
            }
        }
        collector
    }

    /// The repository, for looking up which teams it may act on.
    #[must_use]
    pub fn repo(&self) -> (Provider, &str) {
        match self {
            Self::Push { provider, repo, .. } => (*provider, repo),
            Self::PullRequest { pr, .. } => (pr.provider, &pr.repo),
        }
    }
}
