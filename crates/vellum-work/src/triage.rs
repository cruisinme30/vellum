// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The triage inbox: rules that route new issues, and the four actions.
//!
//! Issues from non-members, forms, Slack and email land in the team's
//! triage status. Rules run top to bottom when an issue enters triage; the
//! first match sets team, assignee, labels and priority. A person then
//! accepts (`1`), declines, marks as duplicate or snoozes each issue.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::WorkError;
use crate::filter::{EvalContext, Filter};
use crate::id::{IssueId, UserRef};
use crate::issue::{Issue, Relation, RelationKind};
use crate::label::group_of;
use crate::scale::Priority;
use crate::team::TeamKey;
use crate::workflow::{Category, StatusKey, Workflow};

/// What a matching triage rule sets.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleActions {
    /// Move the issue to this team's triage inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamKey>,
    /// Assign the issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// Add these labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Set the priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
}

/// One triage rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriageRule {
    /// A name shown in the rule list and the issue's history.
    pub name: String,
    /// Which issues the rule matches.
    pub when: Filter,
    /// What it sets.
    pub then: RuleActions,
}

/// The first rule that matches the issue, if any.
#[must_use]
pub fn first_match<'a>(
    rules: &'a [TriageRule],
    issue: &Issue,
    ctx: &EvalContext<'_>,
) -> Option<&'a TriageRule> {
    rules.iter().find(|r| r.when.matches(issue, ctx))
}

/// Applies a rule's assignee, labels and priority to the issue. A label
/// replaces any label of the same group. The team move is left to the
/// caller, which owns numbering.
pub fn apply_rule(issue: &mut Issue, actions: &RuleActions) {
    if let Some(a) = &actions.assignee {
        issue.assignee = Some(a.clone());
    }
    if let Some(p) = actions.priority {
        issue.priority = p;
    }
    for label in &actions.labels {
        add_label(&mut issue.labels, label);
    }
}

/// Adds a label reference, replacing any label in the same group.
pub fn add_label(labels: &mut Vec<String>, label: &str) {
    if let Some(group) = group_of(label) {
        labels.retain(|l| group_of(l) != Some(group));
    }
    if !labels.iter().any(|l| l.eq_ignore_ascii_case(label)) {
        labels.push(label.to_owned());
    }
}

/// A triage decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TriageAction {
    /// Accept into the team, in `status` or the team's default status.
    Accept {
        /// The status to accept into; must not be triage.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<StatusKey>,
    },
    /// Decline: move to the first canceled status.
    Decline {
        /// An optional reason, posted as a comment by the caller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Close as a duplicate of another issue.
    Duplicate {
        /// The issue this one duplicates.
        of: IssueId,
    },
    /// Hide from the inbox until a time.
    Snooze {
        /// When the issue reappears.
        #[serde(with = "crate::timestamp")]
        until: OffsetDateTime,
    },
}

/// Applies a triage decision. Returns the old and new categories.
///
/// # Errors
///
/// Returns [`WorkError::InvalidState`] if the issue is not in triage or
/// the accept status is a triage status, and [`WorkError::InvalidRelation`]
/// if an issue is marked a duplicate of itself.
pub fn decide(
    issue: &mut Issue,
    action: &TriageAction,
    workflow: &Workflow,
    now: OffsetDateTime,
) -> Result<(Option<Category>, Category), WorkError> {
    let not_in_triage = || WorkError::InvalidState {
        issue: issue.key().to_string(),
        reason: "the issue is not in triage",
    };
    if workflow.category_of(&issue.state) != Some(Category::Triage) {
        return Err(not_in_triage());
    }
    let missing = |c: Category| WorkError::TeamConfig {
        team: issue.team.clone(),
        reason: format!("workflow has no {c} status"),
    };
    match action {
        TriageAction::Accept { status } => {
            let to = status.clone().unwrap_or_else(|| workflow.default.clone());
            if workflow.category_of(&to) == Some(Category::Triage) {
                return Err(WorkError::InvalidState {
                    issue: issue.key().to_string(),
                    reason: "accept needs a status outside triage",
                });
            }
            issue.set_state(to, workflow, now)
        }
        TriageAction::Decline { .. } => {
            let to = workflow
                .first_in(Category::Canceled)
                .ok_or_else(|| missing(Category::Canceled))?
                .key
                .clone();
            issue.set_state(to, workflow, now)
        }
        TriageAction::Duplicate { of } => {
            if of == &issue.id {
                return Err(WorkError::InvalidRelation {
                    issue: issue.key().to_string(),
                    reason: "an issue cannot duplicate itself",
                });
            }
            let to = workflow
                .first_in(Category::Duplicate)
                .ok_or_else(|| missing(Category::Duplicate))?
                .key
                .clone();
            issue
                .relations
                .retain(|r| r.kind != RelationKind::DuplicateOf);
            issue.relations.push(Relation {
                kind: RelationKind::DuplicateOf,
                issue: *of,
            });
            issue.set_state(to, workflow, now)
        }
        TriageAction::Snooze { until } => {
            issue.snoozed_until = Some(*until);
            issue.updated_at = now;
            Ok((Some(Category::Triage), Category::Triage))
        }
    }
}

/// Whether the issue shows in the inbox at `now` (not snoozed).
#[must_use]
pub fn is_visible(issue: &Issue, now: OffsetDateTime) -> bool {
    issue.snoozed_until.is_none_or(|t| t <= now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::tests::Fixed;
    use crate::filter::{Condition, Field, Op};
    use crate::issue::IssueKey;
    use time::macros::{date, datetime};

    fn key(s: &str) -> StatusKey {
        StatusKey::parse(s).unwrap()
    }

    fn triaged() -> Issue {
        let mut i = Issue::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0A1".parse::<IssueId>().unwrap(),
            IssueKey::parse("ENG-5").unwrap(),
            "Crash on login from Slack",
            key("triage"),
            datetime!(2026-10-29 08:00 UTC),
        );
        i.labels = vec!["type/feature".into()];
        i
    }

    #[test]
    fn the_first_matching_rule_wins() {
        let rules: Vec<TriageRule> = serde_saphyr::from_str(
            "- name: crashes\n  when: {field: title, op: contains, values: [crash]}\n  then: {assignee: dev, labels: [type/bug], priority: 1}\n\
             - name: all\n  when: {and: []}\n  then: {assignee: priya}\n",
        )
        .unwrap();
        let mut i = triaged();
        let ctx = EvalContext {
            today: date!(2026 - 10 - 29),
            now: datetime!(2026-10-29 08:00 UTC),
            me: None,
            lookup: &Fixed,
        };
        let rule = first_match(&rules, &i, &ctx).unwrap();
        assert_eq!(rule.name, "crashes");
        apply_rule(&mut i, &rule.then);
        assert_eq!(i.assignee.as_ref().unwrap().as_str(), "dev");
        assert_eq!(i.priority, Priority::Urgent);
        assert_eq!(i.labels, ["type/bug"]);
        let none = [TriageRule {
            name: "x".into(),
            when: Condition::new(Field::Title, Op::Contains, &["nope"]).into(),
            then: RuleActions::default(),
        }];
        assert!(first_match(&none, &i, &ctx).is_none());
    }

    #[test]
    fn the_four_actions() {
        let w = Workflow::standard().unwrap();
        let now = datetime!(2026-10-29 09:00 UTC);

        let mut i = triaged();
        let until = datetime!(2026-10-30 09:00 UTC);
        let _ = decide(&mut i, &TriageAction::Snooze { until }, &w, now).unwrap();
        assert!(!is_visible(&i, now));
        assert!(is_visible(&i, until));

        let _ = decide(&mut i, &TriageAction::Accept { status: None }, &w, now).unwrap();
        assert_eq!(i.state, key("backlog"));
        assert_eq!(i.triaged_at, Some(now));
        assert!(decide(&mut i, &TriageAction::Accept { status: None }, &w, now).is_err());

        let mut i = triaged();
        let bad = TriageAction::Accept {
            status: Some(key("triage")),
        };
        assert!(decide(&mut i, &bad, &w, now).is_err());
        let _ = decide(&mut i, &TriageAction::Decline { reason: None }, &w, now).unwrap();
        assert_eq!(i.state, key("canceled"));

        let mut i = triaged();
        let me = TriageAction::Duplicate { of: i.id };
        assert!(decide(&mut i, &me, &w, now).is_err());
        let of = "01JBZ3M8K2Q4R5S6T7V8W9X0A9".parse::<IssueId>().unwrap();
        let _ = decide(&mut i, &TriageAction::Duplicate { of }, &w, now).unwrap();
        assert_eq!(i.state, key("duplicate"));
        assert_eq!(i.relations[0].issue, of);
        assert!(i.canceled_at.is_some());
    }

    #[test]
    fn actions_serialise_with_a_tag() {
        let a = TriageAction::Accept {
            status: Some(key("todo")),
        };
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, r#"{"action":"accept","status":"todo"}"#);
    }
}
