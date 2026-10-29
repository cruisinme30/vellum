// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AI-suggested triage (R-WK-16).
//!
//! An assistant, reached through MCP or the API, reads a [`TriageBrief`]
//! for an issue in a triage inbox: the issue, the teams it could go to,
//! their labels and members, and open issues with similar titles. It
//! answers with a [`TriageSuggestion`]. The suggestion is checked against
//! the workspace and stored on the issue, in its front matter, so a person
//! sees it in the inbox and accepts it with one click or dismisses it.
//! Nothing is applied until a person accepts: the model only proposes.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{info, instrument};

use crate::error::{Result, WorkError};
use crate::graph::{Changes, Ctx, IssuePatch, WorkGraph};
use crate::id::{IssueId, UserRef};
use crate::issue::Issue;
use crate::scale::Priority;
use crate::team::TeamKey;
use crate::triage::TriageAction;
use crate::workflow::Category;

/// The front matter key a pending suggestion is stored under.
pub const KEY: &str = "triage_suggestion";
/// The longest explanation accepted, in bytes.
pub const MAX_REASON: usize = 2048;
/// How many similar issues a brief lists.
pub const MAX_SIMILAR: usize = 5;
/// How much of the issue body a brief carries, in bytes.
pub const MAX_BRIEF_BODY: usize = 4096;

/// A proposed triage decision. Every field is optional except the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageSuggestion {
    /// The team the issue belongs to, if not the inbox's team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamKey>,
    /// The priority to set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Who should own the issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// An open issue this one repeats; accepting closes it as a duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<IssueId>,
    /// Why, in a sentence or two, shown next to the suggestion.
    pub reason: String,
    /// Which assistant made the suggestion, as it names itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// When the suggestion was made.
    #[serde(with = "crate::timestamp")]
    pub at: OffsetDateTime,
}

/// A team an issue could be routed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamOption {
    /// The team key.
    pub key: TeamKey,
    /// The team name.
    pub name: String,
    /// Labels usable on the team's issues (team and workspace labels).
    pub labels: Vec<String>,
    /// The lead and members, who can be assigned.
    pub people: Vec<UserRef>,
}

/// An open issue whose title looks like the one being triaged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Similar {
    /// The issue ID.
    pub id: IssueId,
    /// The identifier, such as `ENG-142`.
    pub key: String,
    /// The title.
    pub title: String,
    /// Shared title words over all title words, in percent.
    pub overlap: u8,
}

/// Everything an assistant needs to suggest a triage decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageBrief {
    /// The issue ID.
    pub id: IssueId,
    /// The identifier.
    pub key: String,
    /// The title.
    pub title: String,
    /// The start of the description.
    pub body: String,
    /// The team whose inbox holds the issue.
    pub team: TeamKey,
    /// Where it could go.
    pub teams: Vec<TeamOption>,
    /// Open issues with similar titles, closest first.
    pub similar: Vec<Similar>,
}

fn words(title: &str) -> Vec<String> {
    let mut out: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 2)
        .map(str::to_lowercase)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn overlap(a: &[String], b: &[String]) -> u8 {
    let shared = a.iter().filter(|w| b.contains(w)).count();
    let all = a.len() + b.len() - shared;
    if all == 0 {
        return 0;
    }
    u8::try_from(shared * 100 / all).unwrap_or(100)
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

impl WorkGraph {
    fn in_triage(&self, issue: &Issue) -> Result<()> {
        if self.category_of(issue) == Some(Category::Triage) {
            Ok(())
        } else {
            Err(WorkError::InvalidState {
                issue: issue.key().to_string(),
                reason: "is not in triage",
            })
        }
    }

    /// The brief an assistant reads before suggesting a decision.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`], or [`WorkError::InvalidState`]
    /// if the issue is not in triage.
    pub fn triage_brief(&self, id: &IssueId) -> Result<TriageBrief> {
        let issue = self.issue_ref(id)?;
        self.in_triage(issue)?;
        let teams = self
            .teams()
            .map(|t| {
                let mut labels: Vec<String> = t
                    .labels
                    .iter()
                    .chain(self.workspace_labels())
                    .map(crate::label::Label::reference)
                    .collect();
                labels.sort();
                labels.dedup();
                let people = t.lead.iter().chain(&t.members).cloned().collect();
                TeamOption {
                    key: t.key.clone(),
                    name: t.name.clone(),
                    labels,
                    people,
                }
            })
            .collect();
        let mine = words(&issue.title);
        let mut similar: Vec<Similar> = self
            .issues()
            .filter(|o| o.id != *id)
            .filter(|o| {
                self.category_of(o)
                    .is_some_and(|c| !c.is_closed() && c != Category::Triage)
            })
            .filter_map(|o| {
                let score = overlap(&mine, &words(&o.title));
                (score >= 25).then(|| Similar {
                    id: o.id,
                    key: o.key().to_string(),
                    title: o.title.clone(),
                    overlap: score,
                })
            })
            .collect();
        similar.sort_by(|a, b| b.overlap.cmp(&a.overlap).then_with(|| a.key.cmp(&b.key)));
        similar.truncate(MAX_SIMILAR);
        Ok(TriageBrief {
            id: *id,
            key: issue.key().to_string(),
            title: issue.title.clone(),
            body: truncate(&issue.body, MAX_BRIEF_BODY).to_owned(),
            team: issue.team.clone(),
            teams,
            similar,
        })
    }

    /// Checks a suggestion and stores it on the issue for a person to
    /// review. A newer suggestion replaces an older one.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::InvalidState`] if the issue is not in triage,
    /// [`WorkError::UnknownTeam`] or [`WorkError::UnknownIssue`] for an
    /// unknown team or duplicate, and [`WorkError::Invalid`] for a label
    /// the team lacks, an assignee who is not on the team, a duplicate that
    /// is the issue itself or closed, or a missing or oversized reason.
    #[instrument(skip_all, fields(issue = %id))]
    pub fn suggest_triage(
        &mut self,
        id: &IssueId,
        suggestion: TriageSuggestion,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let issue = self.issue_ref(id)?;
        self.in_triage(issue)?;
        let invalid =
            |what: &'static str, value: String, reason: &'static str| WorkError::Invalid {
                what,
                value,
                reason,
            };
        let reason = suggestion.reason.trim();
        if reason.is_empty() || reason.len() > MAX_REASON {
            return Err(invalid(
                "suggestion reason",
                truncate(reason, 40).to_owned(),
                "must be 1 to 2048 bytes",
            ));
        }
        let team = self.team_ref(suggestion.team.as_ref().unwrap_or(&issue.team))?;
        for l in &suggestion.labels {
            self.check_label(team, l)?;
        }
        if let Some(a) = &suggestion.assignee
            && !team.is_member(a)
        {
            return Err(invalid(
                "assignee",
                a.to_string(),
                "is not a member of the team",
            ));
        }
        if let Some(d) = &suggestion.duplicate_of {
            let other = self.issue_ref(d)?;
            if d == id || self.category_of(other).is_none_or(Category::is_closed) {
                return Err(invalid(
                    "duplicate",
                    d.to_string(),
                    "must be another open issue",
                ));
            }
        }
        let stored = TriageSuggestion {
            reason: reason.to_owned(),
            at: ctx.now,
            ..suggestion
        };
        let value = serde_json::to_value(&stored).map_err(|e| WorkError::File {
            path: issue.path(),
            reason: e.to_string(),
        })?;
        let mut issue = issue.clone();
        let _ = issue.extra.insert(KEY.to_owned(), value);
        self.store_issue(issue);
        let mut changes = Changes::default();
        let _ = changes.issues.insert(*id);
        info!(
            monotonic_counter.work_triage_suggestions = 1_u64,
            duplicate = stored.duplicate_of.is_some()
        );
        Ok(changes)
    }

    /// The suggestion waiting on an issue, if any.
    #[must_use]
    pub fn suggestion(&self, id: &IssueId) -> Option<TriageSuggestion> {
        let value = self.issue(id)?.extra.get(KEY)?;
        serde_json::from_value(value.clone()).ok()
    }

    /// Drops the waiting suggestion without applying it.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`].
    pub fn dismiss_suggestion(&mut self, id: &IssueId) -> Result<Changes> {
        let mut issue = self.issue_ref(id)?.clone();
        let mut changes = Changes::default();
        if issue.extra.remove(KEY).is_some() {
            self.store_issue(issue);
            let _ = changes.issues.insert(*id);
            info!(monotonic_counter.work_triage_suggestions_dismissed = 1_u64);
        }
        Ok(changes)
    }

    /// Applies the waiting suggestion as the acting person: closes the
    /// issue as a duplicate, or moves it, sets its fields and accepts it.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::InvalidState`] if there is no suggestion or the
    /// issue left triage, and any error of the commands it runs. On error
    /// nothing is changed.
    #[instrument(skip_all, fields(issue = %id))]
    pub fn accept_suggestion(&mut self, id: &IssueId, ctx: &Ctx) -> Result<Changes> {
        let issue = self.issue_ref(id)?;
        self.in_triage(issue)?;
        let key = issue.key().to_string();
        let suggestion = self.suggestion(id).ok_or(WorkError::InvalidState {
            issue: key,
            reason: "has no triage suggestion",
        })?;
        let backup = self.clone();
        let result = self.apply_suggestion(id, &suggestion, ctx);
        if result.is_err() {
            *self = backup;
        }
        result
    }

    fn apply_suggestion(
        &mut self,
        id: &IssueId,
        s: &TriageSuggestion,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let mut changes = self.dismiss_suggestion(id)?;
        if let Some(of) = s.duplicate_of {
            changes.merge(self.triage(id, &TriageAction::Duplicate { of }, ctx)?);
        } else {
            if let Some(team) = &s.team {
                changes.merge(self.move_issue(id, team, ctx)?);
            }
            let issue = self.issue_ref(id)?;
            let mut labels = issue.labels.clone();
            for l in &s.labels {
                if !labels.iter().any(|m| m.eq_ignore_ascii_case(l)) {
                    labels.push(l.clone());
                }
            }
            let patch = IssuePatch {
                priority: s.priority,
                assignee: s.assignee.clone().map(Some),
                labels: (labels != issue.labels).then_some(labels),
                ..IssuePatch::default()
            };
            if patch != IssuePatch::default() {
                changes.merge(self.update_issue(id, patch, ctx)?);
            }
            // A team with triage off has no triage status; the move has
            // already placed the issue in its normal flow.
            if self.category_of(self.issue_ref(id)?) == Some(Category::Triage) {
                changes.merge(self.triage(id, &TriageAction::Accept { status: None }, ctx)?);
            }
        }
        info!(monotonic_counter.work_triage_suggestions_accepted = 1_u64);
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_jaccard_on_long_words() {
        let a = words("Login fails after password reset");
        let b = words("Password reset email fails");
        assert_eq!(overlap(&a, &b), 50);
        assert_eq!(overlap(&[], &[]), 0);
    }

    #[test]
    fn truncate_keeps_characters_whole() {
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(truncate("abc", 10), "abc");
    }
}
