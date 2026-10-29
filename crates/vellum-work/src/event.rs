// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Events the Work engine emits for webhooks, notifications, Git
//! automation, search and insights.
//!
//! Every command on [`crate::WorkGraph`] returns the events it caused. The
//! service layer stamps them into the kernel's event log and fans them out;
//! integrations read [`IssueSnapshot`] rather than whole issues so their
//! input stays small and stable.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::cycle::CycleKey;
use crate::id::{IssueId, ProjectId, UpdateId, UserRef};
use crate::issue::{Issue, IssueKey};
use crate::project::Health;
use crate::scale::Priority;
use crate::sla::Risk;
use crate::team::TeamKey;
use crate::workflow::{Category, StatusKey};

/// The fields of an issue that integrations need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    /// The issue ID.
    pub id: IssueId,
    /// The identifier, such as `ENG-142`.
    pub key: IssueKey,
    /// The team.
    pub team: TeamKey,
    /// The title.
    pub title: String,
    /// The status key.
    pub state: StatusKey,
    /// The status category.
    pub category: Category,
    /// The priority.
    pub priority: Priority,
    /// The assignee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// When work started.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<OffsetDateTime>,
    /// When it completed.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub completed_at: Option<OffsetDateTime>,
    /// When it was canceled.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub canceled_at: Option<OffsetDateTime>,
}

impl IssueSnapshot {
    /// Takes a snapshot of an issue whose status is in `category`.
    #[must_use]
    pub fn of(issue: &Issue, category: Category) -> Self {
        Self {
            id: issue.id,
            key: issue.key(),
            team: issue.team.clone(),
            title: issue.title.clone(),
            state: issue.state.clone(),
            category,
            priority: issue.priority,
            assignee: issue.assignee.clone(),
            started_at: issue.started_at,
            completed_at: issue.completed_at,
            canceled_at: issue.canceled_at,
        }
    }
}

/// One changed field, with old and new values as display text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    /// The front matter key, such as `assignee`.
    pub field: String,
    /// The old value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The new value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Something that happened in Work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkEvent {
    /// An issue was created.
    IssueCreated {
        /// The new issue.
        issue: IssueSnapshot,
        /// The triage rule that routed it, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        triage_rule: Option<String>,
    },
    /// Fields other than the status changed.
    IssueUpdated {
        /// The issue after the change.
        issue: IssueSnapshot,
        /// What changed.
        changes: Vec<FieldChange>,
    },
    /// The status changed.
    IssueStateChanged {
        /// The issue after the change.
        issue: IssueSnapshot,
        /// The old status.
        from: StatusKey,
        /// The old category.
        from_category: Option<Category>,
    },
    /// The issue moved team and got a new identifier.
    IssueMoved {
        /// The issue after the move.
        issue: IssueSnapshot,
        /// The old identifier, now an alias.
        from: IssueKey,
    },
    /// A triage decision was made.
    IssueTriaged {
        /// The issue after the decision.
        issue: IssueSnapshot,
        /// `accept`, `decline`, `duplicate` or `snooze`.
        action: String,
    },
    /// A cycle was created.
    CycleCreated {
        /// The team.
        team: TeamKey,
        /// The cycle.
        cycle: CycleKey,
    },
    /// A finished cycle's unfinished issues moved on.
    CycleRolledOver {
        /// The team.
        team: TeamKey,
        /// The finished cycle.
        from: CycleKey,
        /// The cycle the issues moved to, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<CycleKey>,
        /// Issues moved to the next cycle.
        moved: Vec<IssueId>,
        /// Issues taken out of the cycle.
        removed: Vec<IssueId>,
    },
    /// An SLA clock started.
    SlaStarted {
        /// The issue.
        issue: IssueSnapshot,
        /// The SLA type.
        kind: String,
        /// The deadline.
        #[serde(with = "crate::timestamp")]
        breaches_at: OffsetDateTime,
    },
    /// An SLA's risk stepped up to orange or red.
    SlaRisk {
        /// The issue.
        issue: IssueSnapshot,
        /// The new risk.
        risk: Risk,
        /// Who to notify.
        recipients: Vec<UserRef>,
    },
    /// An SLA clock stopped because the issue closed.
    SlaStopped {
        /// The issue.
        issue: IssueSnapshot,
        /// Whether it closed in time.
        met: bool,
    },
    /// A project was created.
    ProjectCreated {
        /// The project.
        project: ProjectId,
    },
    /// Important project fields changed.
    ProjectChanged {
        /// The project.
        project: ProjectId,
        /// The changes, as text for the next update.
        changes: Vec<String>,
    },
    /// A project update was posted.
    ProjectUpdatePosted {
        /// The project.
        project: ProjectId,
        /// The update.
        update: UpdateId,
        /// The health it set.
        health: Health,
    },
    /// A label was added, changed or removed.
    LabelChanged {
        /// The owning team; `None` for workspace labels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        team: Option<TeamKey>,
        /// The label reference.
        label: String,
        /// `created`, `updated` or `deleted`.
        change: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_tagged_json() {
        let e = WorkEvent::CycleCreated {
            team: TeamKey::parse("ENG").unwrap(),
            cycle: CycleKey::parse("2027-C04").unwrap(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(
            json,
            r#"{"type":"cycle_created","team":"ENG","cycle":"2027-C04"}"#
        );
        assert_eq!(serde_json::from_str::<WorkEvent>(&json).unwrap(), e);
    }
}
