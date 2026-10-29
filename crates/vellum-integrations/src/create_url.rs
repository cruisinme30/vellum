// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The prefilled create-issue URL.
//!
//! ```text
//! https://vellum.northstar.dev/team/ENG/new?title=Login+fails&priority=high&labels=bug,web&assignee=me
//! ```
//!
//! Opening the URL shows the new-issue form filled in; it never creates an
//! issue by itself, so a link in an email or a README cannot make anyone
//! file something without seeing it (no CSRF by link). Unknown parameters
//! are ignored and reported so the form can show what was dropped.
//!
//! | Parameter | Value |
//! |---|---|
//! | `title` | one line, at most 255 characters |
//! | `description` | Vellum Markdown, at most 32 KiB |
//! | `priority` | `0`–`4`, `p1`–`p4` or a name |
//! | `assignee` | `me`, a user name or an email |
//! | `labels` | comma-separated names (repeatable) |
//! | `estimate` | on the team's scale |
//! | `project` | name or ID |
//! | `cycle` | `current`, `next` or a number |
//! | `state` | a state name |
//! | `due` | `YYYY-MM-DD` |
//! | `parent` | an issue key |
//! | `template` | a template ID |

use std::collections::BTreeSet;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::model::{IssueKey, Priority, is_team_key};

/// Longest accepted title, in characters.
pub const MAX_TITLE: usize = 255;
/// Longest accepted description, in bytes.
pub const MAX_DESCRIPTION: usize = 32 * 1024;

/// Which cycle to put the issue in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleRef {
    /// The team's active cycle.
    Current,
    /// The one after it.
    Next,
    /// A cycle by number.
    Number(u32),
}

/// The values a create URL carries.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Prefill {
    /// The team key.
    pub team: String,
    /// The title.
    pub title: Option<String>,
    /// The description.
    pub description: Option<String>,
    /// The priority.
    pub priority: Option<Priority>,
    /// The assignee as given (`me`, name or email); resolved by the form.
    pub assignee: Option<String>,
    /// Label names.
    pub labels: BTreeSet<String>,
    /// The estimate.
    pub estimate: Option<String>,
    /// The project name or ID.
    pub project: Option<String>,
    /// The cycle.
    pub cycle: Option<CycleRef>,
    /// A state name.
    pub state: Option<String>,
    /// The due date.
    pub due: Option<Date>,
    /// The parent issue.
    pub parent: Option<IssueKey>,
    /// A template to start from.
    pub template: Option<String>,
    /// Parameters that were ignored, with the reason.
    pub ignored: Vec<(String, String)>,
}

/// Why a create URL cannot be used at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreateUrlError {
    /// The path does not name a team: `/team/<KEY>/new`.
    #[error("not a create-issue URL")]
    NotCreateUrl,
}

/// Reads a create URL. Bad values are dropped into [`Prefill::ignored`]
/// rather than failing, so a half-right link still opens the form.
///
/// # Errors
///
/// [`CreateUrlError::NotCreateUrl`] if the path is not `/team/<KEY>/new`.
pub fn parse(url: &Url) -> Result<Prefill, CreateUrlError> {
    let segments: Vec<&str> = url
        .path_segments()
        .map(Iterator::collect)
        .unwrap_or_default();
    let team = match segments.as_slice() {
        ["team", key, "new"] if is_team_key(&key.to_ascii_uppercase()) => key.to_ascii_uppercase(),
        _ => return Err(CreateUrlError::NotCreateUrl),
    };
    let mut prefill = Prefill {
        team,
        ..Prefill::default()
    };
    for (name, value) in url.query_pairs() {
        let value = value.trim().to_owned();
        if value.is_empty() {
            continue;
        }
        let result: Result<(), &str> = match name.as_ref() {
            "title" => {
                if value.chars().count() > MAX_TITLE || value.contains('\n') {
                    Err("title must be one line of at most 255 characters")
                } else {
                    prefill.title = Some(value);
                    Ok(())
                }
            }
            "description" => {
                if value.len() > MAX_DESCRIPTION {
                    Err("description is longer than 32 KiB")
                } else {
                    prefill.description = Some(value);
                    Ok(())
                }
            }
            "priority" => Priority::parse(&value)
                .map(|p| prefill.priority = Some(p))
                .ok_or("unknown priority"),
            "assignee" => {
                prefill.assignee = Some(value);
                Ok(())
            }
            "labels" | "label" => {
                prefill.labels.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned),
                );
                Ok(())
            }
            "estimate" => {
                prefill.estimate = Some(value);
                Ok(())
            }
            "project" => {
                prefill.project = Some(value);
                Ok(())
            }
            "cycle" => match value.to_ascii_lowercase().as_str() {
                "current" => {
                    prefill.cycle = Some(CycleRef::Current);
                    Ok(())
                }
                "next" => {
                    prefill.cycle = Some(CycleRef::Next);
                    Ok(())
                }
                number => number
                    .parse()
                    .map(|n| prefill.cycle = Some(CycleRef::Number(n)))
                    .map_err(|_| "cycle must be current, next or a number"),
            },
            "state" | "status" => {
                prefill.state = Some(value);
                Ok(())
            }
            "due" => value
                .parse::<Date>()
                .map(|d| prefill.due = Some(d))
                .map_err(|_| "due must be YYYY-MM-DD"),
            "parent" => IssueKey::parse(&value)
                .map(|k| prefill.parent = Some(k))
                .map_err(|_| "parent must be an issue key"),
            "template" => {
                prefill.template = Some(value);
                Ok(())
            }
            _ => Err("unknown parameter"),
        };
        if let Err(reason) = result {
            prefill.ignored.push((name.into_owned(), reason.to_owned()));
        }
    }
    Ok(prefill)
}

/// Builds a create URL on `base` (the app's public URL).
#[must_use]
pub fn build(base: &Url, prefill: &Prefill) -> Url {
    let mut url = base.clone();
    url.set_path(&format!("/team/{}/new", prefill.team));
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        let mut put = |name: &str, value: Option<String>| {
            if let Some(value) = value {
                let _ = query.append_pair(name, &value);
            }
        };
        put("title", prefill.title.clone());
        put("description", prefill.description.clone());
        put("priority", prefill.priority.map(|p| p.number().to_string()));
        put("assignee", prefill.assignee.clone());
        let labels: Vec<&str> = prefill.labels.iter().map(String::as_str).collect();
        put("labels", (!labels.is_empty()).then(|| labels.join(",")));
        put("estimate", prefill.estimate.clone());
        put("project", prefill.project.clone());
        put(
            "cycle",
            prefill.cycle.as_ref().map(|c| match c {
                CycleRef::Current => "current".to_owned(),
                CycleRef::Next => "next".to_owned(),
                CycleRef::Number(n) => n.to_string(),
            }),
        );
        put("state", prefill.state.clone());
        put("due", prefill.due.map(|d| d.to_string()));
        put("parent", prefill.parent.as_ref().map(ToString::to_string));
        put("template", prefill.template.clone());
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn parses_every_parameter() -> Result<(), CreateUrlError> {
        let prefill = parse(&url(
            "https://vellum.northstar.dev/team/eng/new?title=Login+fails&description=On+**Safari**\
             &priority=p1&assignee=me&labels=bug,web&label=ios&estimate=3&project=Auth&cycle=next\
             &state=Todo&due=2026-11-24&parent=ENG-1&template=bug&utm_source=x&priority=nope",
        ))?;
        assert_eq!(prefill.team, "ENG");
        assert_eq!(prefill.title.as_deref(), Some("Login fails"));
        assert_eq!(prefill.description.as_deref(), Some("On **Safari**"));
        assert_eq!(prefill.priority, Some(Priority::Urgent));
        assert_eq!(prefill.labels.len(), 3);
        assert_eq!(prefill.cycle, Some(CycleRef::Next));
        assert_eq!(
            prefill.due.map(|d| d.to_string()).as_deref(),
            Some("2026-11-24")
        );
        assert_eq!(
            prefill.parent.map(|k| k.to_string()).as_deref(),
            Some("ENG-1")
        );
        assert_eq!(
            prefill.ignored,
            vec![
                ("utm_source".to_owned(), "unknown parameter".to_owned()),
                ("priority".to_owned(), "unknown priority".to_owned()),
            ]
        );
        Ok(())
    }

    #[test]
    fn rejects_other_paths() {
        assert_eq!(
            parse(&url("https://x.dev/team/ENG/issues")),
            Err(CreateUrlError::NotCreateUrl)
        );
        assert_eq!(
            parse(&url("https://x.dev/team/e-g/new")),
            Err(CreateUrlError::NotCreateUrl)
        );
    }

    #[test]
    fn build_round_trips() -> Result<(), CreateUrlError> {
        let mut prefill = Prefill {
            team: "OPS".to_owned(),
            ..Prefill::default()
        };
        prefill.title = Some("Rotate keys & certs".to_owned());
        prefill.labels = ["sec".to_owned(), "infra".to_owned()].into_iter().collect();
        prefill.cycle = Some(CycleRef::Number(12));
        prefill.priority = Some(Priority::High);
        let built = build(&url("https://vellum.northstar.dev/?x=1"), &prefill);
        assert!(
            built.as_str().starts_with(
                "https://vellum.northstar.dev/team/OPS/new?title=Rotate+keys+%26+certs"
            )
        );
        assert_eq!(parse(&built)?, prefill);
        let empty = build(
            &url("https://vellum.northstar.dev"),
            &Prefill {
                team: "OPS".to_owned(),
                ..Prefill::default()
            },
        );
        assert_eq!(empty.as_str(), "https://vellum.northstar.dev/team/OPS/new");
        Ok(())
    }

    #[test]
    fn long_values_are_ignored() -> Result<(), CreateUrlError> {
        let long = "a".repeat(MAX_TITLE + 1);
        let prefill = parse(&url(&format!("https://x.dev/team/ENG/new?title={long}")))?;
        assert_eq!(prefill.title, None);
        assert_eq!(prefill.ignored.len(), 1);
        Ok(())
    }
}
