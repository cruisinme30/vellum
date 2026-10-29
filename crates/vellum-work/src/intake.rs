// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Intake forms ("Asks"): structured requests that become triage issues
//! (R-WK-16).
//!
//! A team defines forms in `team.yaml`, such as "Request access" or
//! "Report a bug". Anyone allowed to see the form fills it in; the answers
//! become an issue in the team's triage inbox, with the title, priority
//! and labels taken from the fields the form maps, and every answer kept
//! in the description so nothing is lost.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use crate::error::{Result, WorkError};
use crate::graph::{Changes, Ctx, NewIssue, WorkGraph};
use crate::id::IssueId;
use crate::scale::Priority;
use crate::team::TeamKey;

/// The longest answer accepted, in bytes.
pub const MAX_ANSWER: usize = 16 * 1024;

/// What a form field asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FieldKind {
    /// One line of text.
    Text,
    /// Several paragraphs.
    LongText,
    /// One of a fixed list.
    Select {
        /// The choices.
        options: Vec<String>,
    },
    /// An email address.
    Email,
}

/// Where a field's answer goes on the issue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapsTo {
    /// Only into the description's answer list.
    #[default]
    Description,
    /// The issue title.
    Title,
    /// The priority: `urgent`, `high`, `normal`, `low` or `none`.
    Priority,
    /// A label: the answer is the label name.
    Label,
}

/// One question on a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// A slug, unique in the form, such as `summary`.
    pub key: String,
    /// The question shown to people.
    pub label: String,
    /// What kind of answer.
    #[serde(flatten)]
    pub kind: FieldKind,
    /// Whether an answer is needed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Where the answer goes.
    #[serde(default)]
    pub maps_to: MapsTo,
}

/// An intake form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeForm {
    /// A slug, unique in the team, used in the form's URL.
    pub slug: String,
    /// The form's title, such as "Request access".
    pub title: String,
    /// Help shown above the questions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The questions, in order.
    pub fields: Vec<FormField>,
    /// Labels every submission gets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Whether people outside the workspace may submit (through a share
    /// link); otherwise only signed-in members may.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public: bool,
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl IntakeForm {
    /// Checks the form's shape: slugs, at most one title field, options.
    ///
    /// # Errors
    ///
    /// Returns a description of the first problem.
    pub fn check(&self) -> Result<(), String> {
        if !is_slug(&self.slug) {
            return Err(format!(
                "form {:?}: the slug must be lowercase letters, digits and '-'",
                self.slug
            ));
        }
        if self.fields.is_empty() {
            return Err(format!("form {}: needs at least one field", self.slug));
        }
        let mut keys = BTreeSet::new();
        for f in &self.fields {
            if !is_slug(&f.key) || !keys.insert(f.key.as_str()) {
                return Err(format!(
                    "form {}: field key {:?} is invalid or repeated",
                    self.slug, f.key
                ));
            }
            if let FieldKind::Select { options } = &f.kind
                && options.is_empty()
            {
                return Err(format!(
                    "form {}: field {} has no options",
                    self.slug, f.key
                ));
            }
        }
        for target in [MapsTo::Title, MapsTo::Priority] {
            if self.fields.iter().filter(|f| f.maps_to == target).count() > 1 {
                return Err(format!(
                    "form {}: only one field can set the {target:?}",
                    self.slug
                ));
            }
        }
        Ok(())
    }

    /// Turns answers into an issue draft for `team`.
    ///
    /// Without a title field, the title is the form's title and the first
    /// line of the first answer.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] for a missing required answer, an
    /// unknown field, a choice not on the list, a bad email or priority, or
    /// an oversized answer.
    pub fn draft(
        &self,
        id: IssueId,
        team: TeamKey,
        answers: &BTreeMap<String, String>,
        submitter: Option<&str>,
    ) -> Result<NewIssue> {
        let invalid = |field: &str, reason: &'static str| WorkError::Invalid {
            what: "form answer",
            value: field.to_owned(),
            reason,
        };
        if let Some(unknown) = answers
            .keys()
            .find(|k| !self.fields.iter().any(|f| &f.key == *k))
        {
            return Err(invalid(unknown, "is not a field of this form"));
        }
        let mut title = None;
        let mut priority = None;
        let mut labels = self.labels.clone();
        let mut lines = Vec::new();
        for f in &self.fields {
            let answer = answers
                .get(&f.key)
                .map(|a| a.trim())
                .filter(|a| !a.is_empty());
            let Some(answer) = answer else {
                if f.required {
                    return Err(invalid(&f.key, "is required"));
                }
                continue;
            };
            if answer.len() > MAX_ANSWER {
                return Err(invalid(&f.key, "is longer than 16 KiB"));
            }
            match &f.kind {
                FieldKind::Select { options } if !options.iter().any(|o| o == answer) => {
                    return Err(invalid(&f.key, "is not one of the choices"));
                }
                FieldKind::Email if !looks_like_email(answer) => {
                    return Err(invalid(&f.key, "is not an email address"));
                }
                FieldKind::Text | FieldKind::Email if answer.contains('\n') => {
                    return Err(invalid(&f.key, "must be one line"));
                }
                _ => {}
            }
            match f.maps_to {
                MapsTo::Title => title = Some(answer.to_owned()),
                MapsTo::Priority => {
                    priority = Some(
                        priority_named(answer)
                            .ok_or_else(|| invalid(&f.key, "is not a priority"))?,
                    );
                }
                MapsTo::Label => labels.push(answer.to_owned()),
                MapsTo::Description => {}
            }
            if matches!(f.kind, FieldKind::LongText) {
                lines.push(format!("**{}**\n\n{answer}\n", f.label));
            } else {
                lines.push(format!("**{}**: {answer}\n", f.label));
            }
        }
        let title = title.unwrap_or_else(|| {
            let first = lines
                .first()
                .and_then(|l| l.split(": ").nth(1).or_else(|| l.lines().nth(2)))
                .map(|l| {
                    l.lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .chars()
                        .take(80)
                        .collect::<String>()
                })
                .unwrap_or_default();
            if first.is_empty() {
                self.title.clone()
            } else {
                format!("{}: {first}", self.title)
            }
        });
        let mut body = format!("Submitted through the **{}** form", self.title);
        if let Some(who) = submitter {
            body.push_str(" by ");
            body.push_str(who);
        }
        body.push_str(".\n\n");
        body.push_str(&lines.join("\n"));
        let mut draft = NewIssue::new(id, team, &title);
        draft.body = body;
        draft.priority = priority;
        labels.dedup();
        draft.labels = labels;
        draft.intake = true;
        Ok(draft)
    }
}

fn looks_like_email(text: &str) -> bool {
    text.split_once('@').is_some_and(|(user, host)| {
        !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
    }) && !text.chars().any(char::is_whitespace)
}

fn priority_named(text: &str) -> Option<Priority> {
    match text.to_ascii_lowercase().as_str() {
        "urgent" => Some(Priority::Urgent),
        "high" => Some(Priority::High),
        "normal" | "medium" => Some(Priority::Normal),
        "low" => Some(Priority::Low),
        "none" | "no priority" => Some(Priority::None),
        _ => None,
    }
}

impl WorkGraph {
    /// Files a form submission as an issue in the team's triage inbox.
    ///
    /// `ctx.actor` is the signed-in submitter, or `None` for a public form;
    /// `submitter` is how the issue names them (an email for public forms).
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`] or [`WorkError::Invalid`] for an
    /// unknown form, the [`IntakeForm::draft`] errors, and any
    /// [`WorkGraph::create_issue`] error.
    #[instrument(skip_all, fields(team = %team, form = slug))]
    pub fn submit_form(
        &mut self,
        team: &TeamKey,
        slug: &str,
        id: IssueId,
        answers: &BTreeMap<String, String>,
        submitter: Option<&str>,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let form = self
            .team(team)
            .ok_or_else(|| WorkError::UnknownTeam(team.clone()))?
            .forms
            .iter()
            .find(|f| f.slug == slug)
            .cloned()
            .ok_or_else(|| WorkError::Invalid {
                what: "form",
                value: slug.to_owned(),
                reason: "is not a form of the team",
            })?;
        if !form.public && ctx.actor.is_none() {
            return Err(WorkError::Invalid {
                what: "form",
                value: slug.to_owned(),
                reason: "needs a signed-in submitter",
            });
        }
        let draft = form.draft(id, team.clone(), answers, submitter)?;
        let changes = self.create_issue(draft, ctx)?;
        info!(monotonic_counter.work_form_submissions = 1_u64, form = slug);
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access_form() -> IntakeForm {
        IntakeForm {
            slug: "access".into(),
            title: "Request access".into(),
            description: String::new(),
            fields: vec![
                FormField {
                    key: "system".into(),
                    label: "System".into(),
                    kind: FieldKind::Select {
                        options: vec!["Grafana".into(), "Prod DB".into()],
                    },
                    required: true,
                    maps_to: MapsTo::Description,
                },
                FormField {
                    key: "why".into(),
                    label: "Why do you need it?".into(),
                    kind: FieldKind::LongText,
                    required: true,
                    maps_to: MapsTo::Description,
                },
                FormField {
                    key: "urgency".into(),
                    label: "Urgency".into(),
                    kind: FieldKind::Select {
                        options: vec!["High".into(), "Low".into()],
                    },
                    required: false,
                    maps_to: MapsTo::Priority,
                },
                FormField {
                    key: "email".into(),
                    label: "Your email".into(),
                    kind: FieldKind::Email,
                    required: false,
                    maps_to: MapsTo::Description,
                },
            ],
            labels: vec!["access".into()],
            public: false,
        }
    }

    fn answers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn eng() -> TeamKey {
        TeamKey::parse("ENG").unwrap_or_else(|e| unreachable!("{e}"))
    }

    #[test]
    fn answers_become_a_triage_draft() {
        let form = access_form();
        assert_eq!(form.check(), Ok(()));
        let draft = form
            .draft(
                IssueId::generate(),
                eng(),
                &answers(&[
                    ("system", "Grafana"),
                    ("why", "On call\nfrom Monday"),
                    ("urgency", "High"),
                ]),
                Some("sam@northstar.dev"),
            )
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(draft.title, "Request access: Grafana");
        assert_eq!(draft.priority, Some(Priority::High));
        assert_eq!(draft.labels, ["access"]);
        assert!(draft.intake);
        assert!(
            draft
                .body
                .starts_with("Submitted through the **Request access** form by sam@northstar.dev.")
        );
        assert!(
            draft
                .body
                .contains("**Why do you need it?**\n\nOn call\nfrom Monday")
        );
    }

    #[test]
    fn bad_answers_are_refused() {
        let form = access_form();
        let bad = [
            answers(&[("why", "x")]),
            answers(&[("system", "Jira"), ("why", "x")]),
            answers(&[
                ("system", "Grafana"),
                ("why", "x"),
                ("email", "not an email"),
            ]),
            answers(&[("system", "Grafana"), ("why", "x"), ("extra", "y")]),
        ];
        for a in bad {
            assert!(
                form.draft(IssueId::generate(), eng(), &a, None).is_err(),
                "{a:?}"
            );
        }
    }

    #[test]
    fn forms_are_checked() {
        let mut form = access_form();
        form.fields[1].key = "system".into();
        assert!(form.check().is_err());
        let mut form = access_form();
        form.slug = "Access!".into();
        assert!(form.check().is_err());
        let mut form = access_form();
        form.fields[0].maps_to = MapsTo::Priority;
        assert!(form.check().is_err());
    }
}
