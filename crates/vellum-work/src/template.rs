// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Issue templates: field defaults plus a body (R-WK-12).

use serde::{Deserialize, Serialize};

use crate::graph::NewIssue;
use crate::id::{MilestoneId, ProjectId, UserRef};
use crate::scale::{Estimate, Priority};
use crate::triage::add_label;
use crate::workflow::StatusKey;

/// A team's issue template, picked with `Alt C`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// The template name, unique within the team.
    pub name: String,
    /// A title to start from, used when the new issue has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The starting status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StatusKey>,
    /// The priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// The estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    /// The assignee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// Labels added to the issue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// The project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    /// The milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<MilestoneId>,
    /// How often issues made from it repeat, such as `1w` (D224).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<crate::recur::Repeat>,
    /// The body, in Vellum Markdown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

impl Template {
    /// Fills every field the draft leaves unset; labels are added.
    pub fn apply(&self, draft: &mut NewIssue) {
        fn fill<T: Clone>(slot: &mut Option<T>, value: Option<&T>) {
            if slot.is_none() {
                *slot = value.cloned();
            }
        }
        if draft.title.trim().is_empty()
            && let Some(t) = &self.title
        {
            draft.title.clone_from(t);
        }
        fill(&mut draft.state, self.state.as_ref());
        fill(&mut draft.priority, self.priority.as_ref());
        fill(&mut draft.estimate, self.estimate.as_ref());
        fill(&mut draft.assignee, self.assignee.as_ref());
        fill(&mut draft.project, self.project.as_ref());
        fill(&mut draft.milestone, self.milestone.as_ref());
        for l in &self.labels {
            add_label(&mut draft.labels, l);
        }
        if draft.body.trim().is_empty() {
            draft.body.clone_from(&self.body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::IssueId;
    use crate::team::TeamKey;

    #[test]
    fn templates_fill_only_unset_fields() {
        let t: Template = serde_saphyr::from_str(
            "name: Bug report\ntitle: 'Bug: '\npriority: 2\nlabels: [type/bug]\nbody: |\n  ## Steps\n",
        )
        .unwrap();
        let mut d = NewIssue::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0A1".parse::<IssueId>().unwrap(),
            TeamKey::parse("ENG").unwrap(),
            "",
        );
        d.priority = Some(Priority::Urgent);
        d.labels = vec!["type/feature".into(), "gateway".into()];
        t.apply(&mut d);
        assert_eq!(d.title, "Bug: ");
        assert_eq!(d.priority, Some(Priority::Urgent));
        assert_eq!(d.labels, ["gateway", "type/bug"]);
        assert_eq!(d.body, "## Steps\n");
    }
}
