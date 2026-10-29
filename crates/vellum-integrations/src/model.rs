// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The slice of the issue model this crate needs, and the port into the
//! issue core.
//!
//! These types mirror the issue front matter in brief §8. The issue core
//! adapts its own types to them; nothing here is persisted by this crate
//! except the Git links and recurring schedules it owns.

use std::collections::BTreeSet;
use std::fmt;
use std::future::Future;
use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// An issue identifier such as `ENG-142`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct IssueKey {
    /// The team key, upper case (`ENG`).
    pub team: String,
    /// The number within the team.
    pub number: u64,
}

/// Why text is not an issue key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an issue key like ENG-142")]
pub struct InvalidKey(pub String);

/// Whether `team` is a valid team key: a letter, then letters or digits,
/// at most 7 characters, upper case.
#[must_use]
pub fn is_team_key(team: &str) -> bool {
    let mut chars = team.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && team.len() <= 7
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

impl IssueKey {
    /// Parses `ENG-142`, accepting any case (`eng-142` in branch names).
    ///
    /// # Errors
    ///
    /// [`InvalidKey`] for anything else.
    pub fn parse(text: &str) -> Result<Self, InvalidKey> {
        let invalid = || InvalidKey(text.to_owned());
        let (team, number) = text.split_once('-').ok_or_else(invalid)?;
        let team = team.to_ascii_uppercase();
        if !is_team_key(&team)
            || number.is_empty()
            || number.starts_with('0')
            || !number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let number = number.parse().map_err(|_| invalid())?;
        Ok(Self { team, number })
    }
}

impl fmt::Display for IssueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.team, self.number)
    }
}

impl FromStr for IssueKey {
    type Err = InvalidKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for IssueKey {
    type Error = InvalidKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<IssueKey> for String {
    fn from(key: IssueKey) -> Self {
        key.to_string()
    }
}

/// The fixed status categories; each team names its own states inside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateCategory {
    /// Waiting in the team's triage inbox.
    Triage,
    /// Accepted, not planned.
    Backlog,
    /// Planned, not started.
    Unstarted,
    /// In progress.
    Started,
    /// Done.
    Completed,
    /// Will not be done.
    Canceled,
    /// Duplicate of another issue.
    Duplicate,
}

impl StateCategory {
    /// Whether the issue is closed.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Duplicate)
    }
}

/// One of a team's workflow states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowState {
    /// The state's ID.
    pub id: String,
    /// Its name, such as "In Review".
    pub name: String,
    /// Its category.
    pub category: StateCategory,
}

/// Issue priority, 0–4 as stored in front matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// 0: no priority.
    None,
    /// 1: urgent.
    Urgent,
    /// 2: high.
    High,
    /// 3: normal.
    Normal,
    /// 4: low.
    Low,
}

impl Priority {
    /// The stored number.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Urgent => 1,
            Self::High => 2,
            Self::Normal => 3,
            Self::Low => 4,
        }
    }

    /// Parses `0`–`4`, `p0`–`p4` style (p1 = urgent), or a name in any
    /// case, including common synonyms from other trackers.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "0" | "none" | "no priority" | "no_priority" => Some(Self::None),
            "1" | "p1" | "urgent" | "highest" | "blocker" | "critical" | "fatal" => {
                Some(Self::Urgent)
            }
            "2" | "p2" | "high" | "major" => Some(Self::High),
            "3" | "p3" | "normal" | "medium" => Some(Self::Normal),
            "4" | "p4" | "low" | "lowest" | "minor" | "trivial" => Some(Self::Low),
            _ => None,
        }
    }
}

/// What the issue core reports about an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    /// The key.
    pub key: IssueKey,
    /// The title.
    pub title: String,
    /// The current state.
    pub state: WorkflowState,
    /// The assignee's user ID.
    pub assignee: Option<String>,
    /// When it was created.
    pub created_at: Timestamp,
}

/// Where a new issue came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Typed in the app or through the API.
    App,
    /// A Slack slash command or message action.
    Slack {
        /// Permalink to the Slack message, if any.
        permalink: Option<String>,
    },
    /// A Sentry issue alert.
    Sentry {
        /// Sentry's issue ID, used to avoid duplicates.
        issue_id: String,
    },
    /// An email to the team's intake address.
    Email {
        /// The sender's address.
        from: String,
        /// The Message-ID, used to avoid duplicates.
        message_id: String,
    },
    /// An intake form submission.
    Form {
        /// The form's ID.
        form_id: String,
    },
    /// A recurring schedule.
    Recurring {
        /// The schedule's ID.
        schedule_id: String,
    },
    /// An import from another tracker.
    Import {
        /// The tracker (`linear`, `jira`, `github`).
        tracker: String,
        /// The issue's ID there (`ABC-12`, `owner/repo#7`).
        external_id: String,
    },
}

/// A new issue, before the issue core numbers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDraft {
    /// The team key.
    pub team: String,
    /// The title, one line.
    pub title: String,
    /// The description, Vellum Markdown.
    pub description: String,
    /// The priority.
    pub priority: Option<Priority>,
    /// Label names.
    pub labels: BTreeSet<String>,
    /// The assignee's user ID.
    pub assignee: Option<String>,
    /// The estimate on the team's scale.
    pub estimate: Option<String>,
    /// The project's ID or name.
    pub project: Option<String>,
    /// The due date.
    pub due: Option<Date>,
    /// A state ID; `None` means the team's default for the source
    /// (triage for intake, when the team has triage on).
    pub state: Option<String>,
    /// The parent issue.
    pub parent: Option<IssueKey>,
    /// The person who filed it, when known (user ID or email).
    pub reporter: Option<String>,
    /// Where it came from.
    pub source: Source,
}

impl IssueDraft {
    /// An empty draft for `team` from `source`.
    #[must_use]
    pub fn new(team: impl Into<String>, title: impl Into<String>, source: Source) -> Self {
        Self {
            team: team.into(),
            title: title.into(),
            description: String::new(),
            priority: None,
            labels: BTreeSet::new(),
            assignee: None,
            estimate: None,
            project: None,
            due: None,
            state: None,
            parent: None,
            reporter: None,
            source,
        }
    }
}

/// Shortens `text` to one line of at most `max` characters, at a word
/// boundary when possible, adding `…` when cut.
#[must_use]
pub fn one_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let keep = max.saturating_sub(1);
    let cut: String = line.chars().take(keep).collect();
    let at_word_end = line.chars().nth(keep).is_some_and(char::is_whitespace);
    let trimmed = if at_word_end {
        cut.as_str()
    } else {
        cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head)
    };
    format!("{}…", trimmed.trim_end())
}

/// A failure reported by the issue core.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkError {
    /// The issue or team does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The change was refused (permissions, validation).
    #[error("refused: {0}")]
    Refused(String),
    /// A transient failure; the job is retried.
    #[error("unavailable: {0}")]
    Unavailable(String),
}

/// The port into the issue core. Every call is made as the integration's
/// bot user, so authorization and the audit log apply as for a person.
pub trait WorkCommands: Send + Sync {
    /// Looks an issue up by key, following aliases of moved issues.
    fn issue(
        &self,
        key: &IssueKey,
    ) -> impl Future<Output = Result<Option<IssueSnapshot>, WorkError>> + Send;

    /// The workflow states of a team.
    fn states(
        &self,
        team: &str,
    ) -> impl Future<Output = Result<Vec<WorkflowState>, WorkError>> + Send;

    /// Moves an issue to a state, recording why in its history.
    fn set_state(
        &self,
        key: &IssueKey,
        state_id: &str,
        reason: &str,
    ) -> impl Future<Output = Result<(), WorkError>> + Send;

    /// Creates an issue and returns its key.
    fn create_issue(
        &self,
        draft: IssueDraft,
    ) -> impl Future<Output = Result<IssueKey, WorkError>> + Send;

    /// Adds a comment, attributed to `author` (a user ID) when known.
    fn add_comment(
        &self,
        key: &IssueKey,
        author: Option<&str>,
        body: &str,
    ) -> impl Future<Output = Result<(), WorkError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_in_any_case() {
        let key = IssueKey::parse("eng-142");
        assert_eq!(
            key,
            Ok(IssueKey {
                team: "ENG".to_owned(),
                number: 142
            })
        );
        assert_eq!(key.map(|k| k.to_string()), Ok("ENG-142".to_owned()));
        for bad in [
            "ENG",
            "ENG-",
            "ENG-01",
            "1ENG-1",
            "TOOLONGKEY-1",
            "ENG-1a",
            "-1",
            "E_G-1",
        ] {
            assert!(IssueKey::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_serialise_as_strings() -> Result<(), serde_json::Error> {
        let key: IssueKey = serde_json::from_str("\"OPS-7\"")?;
        assert_eq!(serde_json::to_string(&key)?, "\"OPS-7\"");
        assert!(serde_json::from_str::<IssueKey>("\"nope\"").is_err());
        Ok(())
    }

    #[test]
    fn priorities() {
        assert_eq!(Priority::parse("P1"), Some(Priority::Urgent));
        assert_eq!(Priority::parse("Medium"), Some(Priority::Normal));
        assert_eq!(Priority::parse("Trivial"), Some(Priority::Low));
        assert_eq!(Priority::parse("someday"), None);
        assert_eq!(Priority::Low.number(), 4);
    }

    #[test]
    fn one_line_cuts_at_words() {
        assert_eq!(
            one_line("\n  Login fails on Safari\nmore", 80),
            "Login fails on Safari"
        );
        assert_eq!(one_line("Login fails on Safari", 12), "Login fails…");
        assert_eq!(one_line("", 5), "");
    }
}
