// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Intake forms: a team publishes a form (bug report, access request) and
//! each submission files an issue in its triage inbox.
//!
//! A form maps each field to part of the issue: the title, the
//! description, a label, the priority, or a line in a `facts` block at the
//! top of the description. Values are plain text and become text nodes
//! through [`Body`], so a submission cannot inject
//! containers, links or images.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::text::{Body, Fact};
use crate::model::{IssueDraft, Priority, Source};

/// What a field collects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldKind {
    /// One line of text.
    ShortText,
    /// Several lines.
    LongText,
    /// One of the options.
    Select {
        /// The allowed values.
        options: Vec<String>,
    },
    /// An email address.
    Email,
    /// An `http(s)` URL.
    Url,
    /// A number.
    Number,
}

/// Where a field's value goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Target {
    /// The issue title.
    Title,
    /// The description body.
    Description,
    /// A label named by the value (select fields).
    Label,
    /// The priority, parsed from the value.
    Priority,
    /// A `Label: value` line in the description's facts block.
    Fact,
}

/// One field of a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Stable ID, the key in submissions.
    pub id: String,
    /// The label shown to the submitter.
    pub label: String,
    /// What it collects.
    pub kind: FieldKind,
    /// Whether it must be filled.
    pub required: bool,
    /// Where the value goes.
    pub target: Target,
}

/// An intake form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form {
    /// The form's ID.
    pub id: String,
    /// The team it files into.
    pub team: String,
    /// Its fields, in display order.
    pub fields: Vec<Field>,
    /// Labels every submission gets.
    pub labels: Vec<String>,
    /// The priority when no field sets one.
    pub priority: Option<Priority>,
    /// Prefix for the title, such as `[Access]`.
    pub title_prefix: Option<String>,
}

/// A problem with one field of a submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    /// The field's ID.
    pub field: String,
    /// What is wrong.
    pub message: String,
}

/// The longest value accepted for any field, in characters.
pub const MAX_VALUE: usize = 16_384;

fn check(field: &Field, value: &str) -> Result<(), String> {
    if value.chars().count() > MAX_VALUE {
        return Err("is too long".to_owned());
    }
    match &field.kind {
        FieldKind::ShortText if value.contains('\n') => {
            Err("must be one line".to_owned())
        }
        FieldKind::Select { options }
            if !options.iter().any(|o| o == value) =>
        {
            Err("is not one of the options".to_owned())
        }
        FieldKind::Email
            if !value.split_once('@').is_some_and(|(l, d)| {
                !l.is_empty()
                    && d.contains('.')
                    && !value.contains(char::is_whitespace)
            }) =>
        {
            Err("is not an email address".to_owned())
        }
        FieldKind::Url
            if !url::Url::parse(value).is_ok_and(|u| {
                u.scheme() == "https" || u.scheme() == "http"
            }) =>
        {
            Err("is not a web address".to_owned())
        }
        FieldKind::Number
            if value.parse::<f64>().map_or(true, |n| !n.is_finite()) =>
        {
            Err("is not a number".to_owned())
        }
        _ => Ok(()),
    }
}

impl Form {
    /// Validates a submission and builds the draft.
    ///
    /// # Errors
    ///
    /// Every [`FieldError`] found, so the form can show them all at once.
    pub fn submit(
        &self,
        values: &BTreeMap<String, String>,
        reporter: Option<String>,
    ) -> Result<IssueDraft, Vec<FieldError>> {
        let mut errors = Vec::new();
        let mut draft = IssueDraft::new(
            self.team.clone(),
            String::new(),
            Source::Form {
                form_id: self.id.clone(),
            },
        );
        draft.labels.extend(self.labels.iter().cloned());
        draft.priority = self.priority;
        draft.reporter = reporter;
        let mut facts = Vec::new();
        let mut body = Vec::new();
        for field in &self.fields {
            let value = values.get(&field.id).map_or("", |v| v.trim());
            if value.is_empty() {
                if field.required {
                    errors.push(FieldError {
                        field: field.id.clone(),
                        message: "is required".to_owned(),
                    });
                }
                continue;
            }
            if let Err(message) = check(field, value) {
                errors.push(FieldError {
                    field: field.id.clone(),
                    message,
                });
                continue;
            }
            match field.target {
                Target::Title => value.clone_into(&mut draft.title),
                Target::Description => body.push(value),
                Target::Label => {
                    let _ = draft.labels.insert(value.to_owned());
                }
                Target::Priority => match Priority::parse(value) {
                    Some(priority) => draft.priority = Some(priority),
                    None => errors.push(FieldError {
                        field: field.id.clone(),
                        message: "is not a priority".to_owned(),
                    }),
                },
                Target::Fact => {
                    facts.push(Fact::new(field.label.clone(), value))
                }
            }
        }
        if draft.title.is_empty() && errors.is_empty() {
            errors.push(FieldError {
                field: "title".to_owned(),
                message: "the form has no title field".to_owned(),
            });
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        if let Some(prefix) = &self.title_prefix {
            draft.title = format!("{prefix} {}", draft.title);
        }
        let mut description = Body::new();
        let _ = description.facts(&facts);
        for text in body {
            let _ = description.text(text).text("");
        }
        draft.description = description.finish();
        Ok(draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(
        id: &str,
        kind: FieldKind,
        required: bool,
        target: Target,
    ) -> Field {
        Field {
            id: id.to_owned(),
            label: id.to_owned(),
            kind,
            required,
            target,
        }
    }

    fn bug_form() -> Form {
        Form {
            id: "bug".to_owned(),
            team: "ENG".to_owned(),
            fields: vec![
                field("summary", FieldKind::ShortText, true, Target::Title),
                field(
                    "details",
                    FieldKind::LongText,
                    true,
                    Target::Description,
                ),
                field(
                    "area",
                    FieldKind::Select {
                        options: vec!["web".to_owned(), "api".to_owned()],
                    },
                    false,
                    Target::Label,
                ),
                field(
                    "severity",
                    FieldKind::Select {
                        options: vec!["high".to_owned(), "low".to_owned()],
                    },
                    false,
                    Target::Priority,
                ),
                field("contact", FieldKind::Email, false, Target::Fact),
                field("page", FieldKind::Url, false, Target::Fact),
            ],
            labels: vec!["bug".to_owned()],
            priority: Some(Priority::Normal),
            title_prefix: Some("[Bug]".to_owned()),
        }
    }

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn a_valid_submission_files_a_draft() -> Result<(), Vec<FieldError>> {
        let draft = bug_form().submit(
            &values(&[
                ("summary", "Export button does nothing"),
                (
                    "details",
                    "Click **Export**\n::: crit\n[x](javascript:alert(1))",
                ),
                ("area", "web"),
                ("severity", "high"),
                ("contact", "zoe@northstar.dev"),
                ("page", "https://vellum.northstar.dev/doc/1"),
            ]),
            None,
        )?;
        assert_eq!(draft.title, "[Bug] Export button does nothing");
        assert_eq!(draft.priority, Some(Priority::High));
        assert_eq!(
            draft.labels.iter().map(String::as_str).collect::<Vec<_>>(),
            ["bug", "web"]
        );
        assert!(
            draft
                .description
                .starts_with("::: facts\n\n- contact: zoe\\@northstar.dev\n")
        );
        assert!(draft.description.contains("Click \\*\\*Export\\*\\*"));
        assert!(draft.description.contains("\\::: crit\\\n"));
        assert!(draft.description.contains("\\[x\\](javascript:alert(1))"));
        Ok(())
    }

    #[test]
    fn errors_are_reported_together() {
        let errors = bug_form()
            .submit(
                &values(&[
                    ("summary", "two\nlines"),
                    ("area", "mobile"),
                    ("contact", "nope"),
                ]),
                None,
            )
            .err()
            .unwrap_or_default();
        let fields: Vec<&str> =
            errors.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields, ["summary", "details", "area", "contact"]);
    }
}
