// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The per-team state mapping and the "all pull requests must merge" rule.
//!
//! Each team maps five Git moments to one of its workflow states, or to
//! nothing:
//!
//! | Moment | Default state |
//! |---|---|
//! | Branch pushed, or draft pull request | first *started* state |
//! | Pull request opened | started state named like "review" |
//! | Pull request approved | started state named like "ready" or "merge" |
//! | Pull request merged | first *completed* state |
//! | Closed without merging | first *unstarted* state |
//!
//! An issue may have several closing pull requests. Its stage is the
//! **least advanced** of the ones still open, so it reaches *completed*
//! only when every closing pull request has merged; pull requests closed
//! unmerged drop out (D79). Automation never reopens a closed issue, never
//! moves an issue backwards past where it is, and only moves an issue back
//! after "closed unmerged" if automation put it where it is.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::event::{PrRef, PullRequestAction};
use super::refs::RefKind;
use crate::model::{IssueSnapshot, StateCategory, WorkflowState};

/// How far the work behind an issue has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Every closing pull request was closed without merging.
    ClosedUnmerged,
    /// A branch was pushed or a draft pull request is open.
    Branch,
    /// A pull request is open for review.
    InReview,
    /// Every open pull request is approved.
    ReadyToMerge,
    /// Every closing pull request merged.
    Merged,
}

/// A team's mapping from stages to its workflow state IDs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mapping {
    /// For [`Stage::Branch`].
    pub branch_pushed: Option<String>,
    /// For [`Stage::InReview`].
    pub pr_opened: Option<String>,
    /// For [`Stage::ReadyToMerge`].
    pub pr_approved: Option<String>,
    /// For [`Stage::Merged`].
    pub pr_merged: Option<String>,
    /// For [`Stage::ClosedUnmerged`].
    pub pr_closed_unmerged: Option<String>,
}

impl Mapping {
    /// The default mapping for a team's states, listed in board order.
    #[must_use]
    pub fn default_for(states: &[WorkflowState]) -> Self {
        let first = |category: StateCategory| {
            states
                .iter()
                .find(|s| s.category == category)
                .map(|s| s.id.clone())
        };
        let started_named = |words: &[&str]| {
            states
                .iter()
                .filter(|s| s.category == StateCategory::Started)
                .find(|s| {
                    let name = s.name.to_lowercase();
                    words.iter().any(|w| name.contains(w))
                })
                .map(|s| s.id.clone())
        };
        let branch = first(StateCategory::Started);
        let review = started_named(&["review"]).or_else(|| branch.clone());
        let ready = started_named(&["ready", "merge"]).or_else(|| review.clone());
        Self {
            branch_pushed: branch,
            pr_opened: review,
            pr_approved: ready,
            pr_merged: first(StateCategory::Completed),
            pr_closed_unmerged: first(StateCategory::Unstarted),
        }
    }

    /// The state for a stage, if mapped.
    #[must_use]
    pub fn state_for(&self, stage: Stage) -> Option<&str> {
        match stage {
            Stage::ClosedUnmerged => self.pr_closed_unmerged.as_deref(),
            Stage::Branch => self.branch_pushed.as_deref(),
            Stage::InReview => self.pr_opened.as_deref(),
            Stage::ReadyToMerge => self.pr_approved.as_deref(),
            Stage::Merged => self.pr_merged.as_deref(),
        }
    }

    /// The most advanced in-progress stage mapped to `state_id`.
    fn progress_stage_of(&self, state_id: &str) -> Option<Stage> {
        [Stage::ReadyToMerge, Stage::InReview, Stage::Branch]
            .into_iter()
            .find(|stage| self.state_for(*stage) == Some(state_id))
    }
}

/// A pull request's state, as far as automation cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrStatus {
    /// A draft.
    Draft,
    /// Open for review.
    Open,
    /// Approved and open.
    Approved,
    /// Merged.
    Merged,
    /// Closed without merging.
    ClosedUnmerged,
}

impl PrStatus {
    /// The status after `action`, from the previous one if any.
    #[must_use]
    pub const fn after(previous: Option<Self>, action: PullRequestAction, draft: bool) -> Self {
        match action {
            PullRequestAction::Draft => Self::Draft,
            PullRequestAction::Opened | PullRequestAction::ReadyForReview => Self::Open,
            PullRequestAction::Approved => Self::Approved,
            PullRequestAction::ChangesRequested => {
                if draft {
                    Self::Draft
                } else {
                    Self::Open
                }
            }
            PullRequestAction::Merged => Self::Merged,
            PullRequestAction::ClosedUnmerged => Self::ClosedUnmerged,
            PullRequestAction::Edited => match previous {
                Some(status) => status,
                None if draft => Self::Draft,
                None => Self::Open,
            },
        }
    }
}

/// One pull request linked to an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrLink {
    /// Which pull request.
    pub pr: PrRef,
    /// Closing or linking.
    pub kind: RefKind,
    /// Its state.
    pub status: PrStatus,
    /// Its title.
    pub title: String,
    /// Link to it.
    pub url: String,
}

/// Everything Git has linked to one issue. Stored by this service.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IssueLinks {
    /// Linked pull requests.
    pub prs: Vec<PrLink>,
    /// Branches pushed for the issue (`repo:branch`).
    pub branches: BTreeSet<String>,
    /// Commits on a default branch whose message closes the issue.
    pub closing_commits: BTreeSet<String>,
    /// Commits on other branches whose message closes the issue.
    pub commits: BTreeSet<String>,
    /// Commits that only link to the issue; they never move it.
    pub linked_commits: BTreeSet<String>,
}

impl IssueLinks {
    /// Inserts or updates a pull request link.
    pub fn upsert(&mut self, link: PrLink) {
        match self.prs.iter_mut().find(|l| l.pr == link.pr) {
            Some(existing) => *existing = link,
            None => self.prs.push(link),
        }
    }

    /// Removes a pull request that no longer mentions the issue.
    pub fn unlink(&mut self, pr: &PrRef) -> bool {
        let before = self.prs.len();
        self.prs.retain(|l| &l.pr != pr);
        self.prs.len() != before
    }

    /// The status a pull request had before this event.
    #[must_use]
    pub fn status_of(&self, pr: &PrRef) -> Option<PrStatus> {
        self.prs.iter().find(|l| &l.pr == pr).map(|l| l.status)
    }

    /// The stage the links put the issue in, if any.
    #[must_use]
    pub fn stage(&self) -> Option<Stage> {
        let closing: Vec<PrStatus> = self
            .prs
            .iter()
            .filter(|l| l.kind == RefKind::Closing)
            .map(|l| l.status)
            .collect();
        let open = closing
            .iter()
            .filter_map(|status| match status {
                PrStatus::Draft => Some(Stage::Branch),
                PrStatus::Open => Some(Stage::InReview),
                PrStatus::Approved => Some(Stage::ReadyToMerge),
                PrStatus::Merged | PrStatus::ClosedUnmerged => None,
            })
            .min();
        if open.is_some() {
            return open;
        }
        if closing.contains(&PrStatus::Merged) || !self.closing_commits.is_empty() {
            return Some(Stage::Merged);
        }
        if closing.contains(&PrStatus::ClosedUnmerged) {
            return Some(Stage::ClosedUnmerged);
        }
        (!self.branches.is_empty() || !self.commits.is_empty()).then_some(Stage::Branch)
    }
}

/// The state change automation makes, if any.
#[must_use]
pub fn decide<'m>(
    issue: &IssueSnapshot,
    stage: Option<Stage>,
    mapping: &'m Mapping,
) -> Option<&'m str> {
    let stage = stage?;
    let target = mapping.state_for(stage)?;
    let current = &issue.state;
    if current.category.is_closed() || current.id == target {
        return None;
    }
    let not_started = matches!(
        current.category,
        StateCategory::Triage | StateCategory::Backlog | StateCategory::Unstarted
    );
    // An unmapped started state (say "QA", set by hand) ranks as Branch.
    let current_stage = mapping
        .progress_stage_of(&current.id)
        .or_else(|| (current.category == StateCategory::Started).then_some(Stage::Branch));
    let allowed = match stage {
        Stage::Merged => true,
        Stage::ClosedUnmerged => mapping.progress_stage_of(&current.id).is_some(),
        Stage::Branch => not_started,
        Stage::InReview | Stage::ReadyToMerge => not_started || current_stage < Some(stage),
    };
    allowed.then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::event::Provider;
    use crate::model::IssueKey;
    use jiff::Timestamp;

    fn state(id: &str, name: &str, category: StateCategory) -> WorkflowState {
        WorkflowState {
            id: id.to_owned(),
            name: name.to_owned(),
            category,
        }
    }

    fn states() -> Vec<WorkflowState> {
        vec![
            state("tri", "Triage", StateCategory::Triage),
            state("bkl", "Backlog", StateCategory::Backlog),
            state("todo", "Todo", StateCategory::Unstarted),
            state("prog", "In Progress", StateCategory::Started),
            state("rev", "In Review", StateCategory::Started),
            state("rdy", "Ready to merge", StateCategory::Started),
            state("qa", "QA", StateCategory::Started),
            state("done", "Done", StateCategory::Completed),
            state("cxl", "Canceled", StateCategory::Canceled),
        ]
    }

    fn issue_in(id: &str) -> IssueSnapshot {
        let state = states()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| unreachable!());
        IssueSnapshot {
            key: IssueKey {
                team: "ENG".to_owned(),
                number: 1,
            },
            title: "t".to_owned(),
            state,
            assignee: None,
            created_at: Timestamp::UNIX_EPOCH,
        }
    }

    fn pr(number: u64, kind: RefKind, status: PrStatus) -> PrLink {
        PrLink {
            pr: PrRef {
                provider: Provider::GitHub,
                repo: "northstar/api".to_owned(),
                number,
            },
            kind,
            status,
            title: String::new(),
            url: String::new(),
        }
    }

    #[test]
    fn default_mapping() {
        let mapping = Mapping::default_for(&states());
        assert_eq!(
            mapping,
            Mapping {
                branch_pushed: Some("prog".to_owned()),
                pr_opened: Some("rev".to_owned()),
                pr_approved: Some("rdy".to_owned()),
                pr_merged: Some("done".to_owned()),
                pr_closed_unmerged: Some("todo".to_owned()),
            }
        );
        // A minimal workflow maps review and ready to the one started state.
        let minimal = Mapping::default_for(&[
            state("t", "Todo", StateCategory::Unstarted),
            state("d", "Doing", StateCategory::Started),
            state("x", "Done", StateCategory::Completed),
        ]);
        assert_eq!(minimal.pr_approved.as_deref(), Some("d"));
    }

    #[test]
    fn all_closing_prs_must_merge() {
        let mut links = IssueLinks::default();
        links.upsert(pr(1, RefKind::Closing, PrStatus::Merged));
        links.upsert(pr(2, RefKind::Closing, PrStatus::Open));
        assert_eq!(links.stage(), Some(Stage::InReview));
        links.upsert(pr(2, RefKind::Closing, PrStatus::Approved));
        assert_eq!(links.stage(), Some(Stage::ReadyToMerge));
        links.upsert(pr(3, RefKind::Closing, PrStatus::Draft));
        assert_eq!(links.stage(), Some(Stage::Branch));
        links.upsert(pr(3, RefKind::Closing, PrStatus::ClosedUnmerged));
        links.upsert(pr(2, RefKind::Closing, PrStatus::Merged));
        assert_eq!(links.stage(), Some(Stage::Merged));
    }

    #[test]
    fn linking_prs_never_drive_state() {
        let mut links = IssueLinks::default();
        links.upsert(pr(1, RefKind::Linking, PrStatus::Merged));
        assert_eq!(links.stage(), None);
        let _ = links.branches.insert("northstar/api:ada/eng-1".to_owned());
        assert_eq!(links.stage(), Some(Stage::Branch));
        assert!(links.unlink(&pr(1, RefKind::Linking, PrStatus::Merged).pr));
        assert!(!links.unlink(&pr(9, RefKind::Linking, PrStatus::Merged).pr));
    }

    #[test]
    fn closed_unmerged_and_closing_commits() {
        let mut links = IssueLinks::default();
        links.upsert(pr(1, RefKind::Closing, PrStatus::ClosedUnmerged));
        assert_eq!(links.stage(), Some(Stage::ClosedUnmerged));
        let _ = links.closing_commits.insert("abc".to_owned());
        assert_eq!(links.stage(), Some(Stage::Merged));
    }

    #[test]
    fn decisions_move_forward_only() {
        let m = Mapping::default_for(&states());
        assert_eq!(
            decide(&issue_in("todo"), Some(Stage::Branch), &m),
            Some("prog")
        );
        assert_eq!(
            decide(&issue_in("tri"), Some(Stage::InReview), &m),
            Some("rev")
        );
        assert_eq!(
            decide(&issue_in("prog"), Some(Stage::InReview), &m),
            Some("rev")
        );
        assert_eq!(
            decide(&issue_in("rev"), Some(Stage::ReadyToMerge), &m),
            Some("rdy")
        );
        // Not backwards.
        assert_eq!(decide(&issue_in("rdy"), Some(Stage::InReview), &m), None);
        assert_eq!(decide(&issue_in("rev"), Some(Stage::Branch), &m), None);
        // A hand-set started state keeps a new branch from moving it.
        assert_eq!(decide(&issue_in("qa"), Some(Stage::Branch), &m), None);
        assert_eq!(
            decide(&issue_in("qa"), Some(Stage::InReview), &m),
            Some("rev")
        );
        // Merged completes from anywhere open; closed issues stay closed.
        assert_eq!(
            decide(&issue_in("qa"), Some(Stage::Merged), &m),
            Some("done")
        );
        assert_eq!(decide(&issue_in("cxl"), Some(Stage::Merged), &m), None);
        assert_eq!(decide(&issue_in("done"), Some(Stage::Branch), &m), None);
        // Closed unmerged only undoes automation's own move.
        assert_eq!(
            decide(&issue_in("rev"), Some(Stage::ClosedUnmerged), &m),
            Some("todo")
        );
        assert_eq!(
            decide(&issue_in("qa"), Some(Stage::ClosedUnmerged), &m),
            None
        );
        assert_eq!(decide(&issue_in("todo"), None, &m), None);
        // An unmapped moment does nothing.
        let off = Mapping {
            pr_opened: None,
            ..m
        };
        assert_eq!(decide(&issue_in("todo"), Some(Stage::InReview), &off), None);
    }

    #[test]
    fn status_transitions() {
        use PullRequestAction as A;
        assert_eq!(PrStatus::after(None, A::Edited, true), PrStatus::Draft);
        assert_eq!(
            PrStatus::after(Some(PrStatus::Approved), A::Edited, false),
            PrStatus::Approved
        );
        assert_eq!(
            PrStatus::after(Some(PrStatus::Approved), A::ChangesRequested, false),
            PrStatus::Open
        );
        assert_eq!(
            PrStatus::after(None, A::ReadyForReview, false),
            PrStatus::Open
        );
    }
}
