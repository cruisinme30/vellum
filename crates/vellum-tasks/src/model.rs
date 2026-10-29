// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The task model: one shape for every source My Work shows.
//!
//! Dates are civil (floating) values in the owner's time zone, the way
//! Things and Todoist store them: "due 3 pm" stays 3 pm when the owner
//! travels. Only reminders and calendar output turn them into instants,
//! using the owner's zone at that moment.

use std::fmt;
use std::str::FromStr;

use jiff::civil::{Date, DateTime};
use jiff::{SignedDuration, Span, SpanRelativeTo, Timestamp};
use vellum_kernel::{BlockId, CommentId, DocId, IssueId};

use crate::attrs::Attrs;
use crate::recur::{RRule, RRuleError};

/// A deadline: a whole day, or a day and a time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Due {
    /// Due some time on this day.
    Date(Date),
    /// Due at this local time.
    DateTime(DateTime),
}

impl Due {
    /// The calendar day of the deadline.
    #[must_use]
    pub fn date(self) -> Date {
        match self {
            Self::Date(date) => date,
            Self::DateTime(datetime) => datetime.date(),
        }
    }

    /// The local time, when one is set.
    #[must_use]
    pub const fn datetime(self) -> Option<DateTime> {
        match self {
            Self::Date(_) => None,
            Self::DateTime(datetime) => Some(datetime),
        }
    }

    /// Moves the deadline to another day, keeping any time of day.
    #[must_use]
    pub fn on(self, date: Date) -> Self {
        match self {
            Self::Date(_) => Self::Date(date),
            Self::DateTime(datetime) => Self::DateTime(date.to_datetime(datetime.time())),
        }
    }
}

/// Why a field value could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FieldError {
    /// A date or date-time is not `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS]`.
    #[error("invalid date {0:?}")]
    Date(String),
    /// A reminder is neither an ISO 8601 duration nor a local date-time.
    #[error("invalid reminder {0:?}")]
    Reminder(String),
    /// An unknown `when`, `recur_from` or priority value.
    #[error("invalid value {value:?} for {field}")]
    Value {
        /// The field name.
        field: &'static str,
        /// The rejected value.
        value: String,
    },
    /// The recurrence rule is not valid.
    #[error(transparent)]
    RRule(#[from] RRuleError),
}

impl FromStr for Due {
    type Err = FieldError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.contains('T') {
            text.parse::<DateTime>()
                .map(Self::DateTime)
                .map_err(|_| FieldError::Date(text.to_owned()))
        } else {
            text.parse::<Date>()
                .map(Self::Date)
                .map_err(|_| FieldError::Date(text.to_owned()))
        }
    }
}

impl fmt::Display for Due {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(date) => write!(f, "{date}"),
            Self::DateTime(datetime) => {
                let time = datetime.time();
                if time.second() == 0 && time.subsec_nanosecond() == 0 {
                    write!(f, "{}T{}", datetime.date(), time.strftime("%H:%M"))
                } else {
                    write!(f, "{}T{}", datetime.date(), time.strftime("%H:%M:%S"))
                }
            }
        }
    }
}

/// An explicit placement that dates alone cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// Today, after work: the "This evening" part of Today.
    Evening,
    /// Ready to do whenever; leaves the Inbox without a date.
    Anytime,
    /// Parked; not shown until the owner looks.
    Someday,
}

impl When {
    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Evening => "evening",
            Self::Anytime => "anytime",
            Self::Someday => "someday",
        }
    }
}

impl FromStr for When {
    type Err = FieldError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "evening" => Ok(Self::Evening),
            "anytime" => Ok(Self::Anytime),
            "someday" => Ok(Self::Someday),
            other => Err(FieldError::Value {
                field: "when",
                value: other.to_owned(),
            }),
        }
    }
}

/// Where the next occurrence of a repeating task is counted from (R-MW-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecurFrom {
    /// From the schedule: "every Monday" stays on Mondays (`every`).
    #[default]
    Schedule,
    /// From the day it was completed (`every!`).
    Completion,
}

impl RecurFrom {
    /// The stored spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Completion => "completion",
        }
    }
}

impl FromStr for RecurFrom {
    type Err = FieldError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "schedule" => Ok(Self::Schedule),
            "completion" => Ok(Self::Completion),
            other => Err(FieldError::Value {
                field: "recur_from",
                value: other.to_owned(),
            }),
        }
    }
}

/// A repeating schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    /// The RFC 5545 rule.
    pub rule: RRule,
    /// What the next occurrence is counted from.
    pub from: RecurFrom,
}

/// When to remind the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reminder {
    /// Relative to the deadline (a date-only deadline counts from the
    /// digest hour), written as an ISO 8601 duration such as `-PT15M`.
    Relative(SignedDuration),
    /// At a fixed local time.
    At(DateTime),
}

impl FromStr for Reminder {
    type Err = FieldError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let unsigned = text.trim_start_matches(['-', '+']);
        if unsigned.starts_with('P') || unsigned.starts_with('p') {
            text.parse::<Span>()
                .ok()
                .and_then(|span| span.to_duration(SpanRelativeTo::days_are_24_hours()).ok())
                .map(Self::Relative)
                .ok_or_else(|| FieldError::Reminder(text.to_owned()))
        } else {
            text.parse::<DateTime>()
                .map(Self::At)
                .map_err(|_| FieldError::Reminder(text.to_owned()))
        }
    }
}

impl fmt::Display for Reminder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Relative(duration) => write!(f, "{duration}"),
            Self::At(datetime) => write!(f, "{}", Due::DateTime(*datetime)),
        }
    }
}

/// Priority 1 (highest) to 4 (lowest), as in `p1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority(u8);

impl Priority {
    /// Builds a priority from 1 to 4.
    #[must_use]
    pub const fn new(level: u8) -> Option<Self> {
        if level >= 1 && level <= 4 {
            Some(Self(level))
        } else {
            None
        }
    }

    /// The level, 1 to 4.
    #[must_use]
    pub const fn level(self) -> u8 {
        self.0
    }
}

/// The structured fields of a task, as stored in its attribute span.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFields {
    /// An explicit placement.
    pub when: Option<When>,
    /// When the owner plans to begin (R-MW-2).
    pub start: Option<Date>,
    /// The deadline, with an optional time (R-MW-2).
    pub due: Option<Due>,
    /// The repeat schedule (R-MW-4).
    pub recurrence: Option<Recurrence>,
    /// Reminders (R-MW-5).
    pub reminders: Vec<Reminder>,
    /// Priority.
    pub priority: Option<Priority>,
    /// Free-form labels.
    pub labels: Vec<String>,
    /// When it was completed.
    pub done: Option<Timestamp>,
    /// Where an imported or captured task came from, such as
    /// `todoist:6Xr8`, so a re-import skips it.
    pub src: Option<String>,
}

fn split_list(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty())
}

impl TaskFields {
    /// Reads the fields from an attribute span; unknown keys are ignored
    /// here and kept by the span itself.
    ///
    /// # Errors
    ///
    /// Returns the first invalid value.
    pub fn from_attrs(attrs: &Attrs) -> Result<Self, FieldError> {
        let mut fields = Self {
            when: attrs.get("when").map(str::parse).transpose()?,
            start: attrs
                .get("start")
                .map(|v| {
                    v.parse::<Date>()
                        .map_err(|_| FieldError::Date(v.to_owned()))
                })
                .transpose()?,
            due: attrs.get("due").map(str::parse).transpose()?,
            ..Self::default()
        };
        if let Some(rule) = attrs.get("rrule") {
            let from = attrs
                .get("recur_from")
                .map(str::parse)
                .transpose()?
                .unwrap_or_default();
            fields.recurrence = Some(Recurrence {
                rule: rule.parse()?,
                from,
            });
        }
        if let Some(reminders) = attrs.get("remind") {
            fields.reminders = split_list(reminders)
                .map(str::parse)
                .collect::<Result<_, _>>()?;
        }
        if let Some(p) = attrs.get("p") {
            fields.priority = p.parse::<u8>().ok().and_then(Priority::new);
            if fields.priority.is_none() {
                return Err(FieldError::Value {
                    field: "p",
                    value: p.to_owned(),
                });
            }
        }
        if let Some(labels) = attrs.get("labels") {
            fields.labels = split_list(labels).map(str::to_owned).collect();
        }
        if let Some(done) = attrs.get("done") {
            fields.done = Some(
                done.parse::<Timestamp>()
                    .map_err(|_| FieldError::Date(done.to_owned()))?,
            );
        }
        fields.src = attrs.get("src").map(str::to_owned);
        Ok(fields)
    }

    /// Writes the fields into an attribute span, keeping unknown keys.
    pub fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.set_opt("when", self.when.map(When::as_str));
        attrs.set_opt("start", self.start.map(|d| d.to_string()));
        attrs.set_opt("due", self.due.map(|d| d.to_string()));
        attrs.set_opt(
            "rrule",
            self.recurrence.as_ref().map(|r| r.rule.to_string()),
        );
        attrs.set_opt(
            "recur_from",
            self.recurrence
                .as_ref()
                .filter(|r| r.from == RecurFrom::Completion)
                .map(|r| r.from.as_str()),
        );
        let reminders: Vec<String> = self.reminders.iter().map(ToString::to_string).collect();
        attrs.set("remind", reminders.join(","));
        attrs.set_opt("p", self.priority.map(|p| p.level().to_string()));
        attrs.set("labels", self.labels.join(","));
        attrs.set_opt(
            "done",
            self.done
                .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
        );
        attrs.set_opt("src", self.src.clone());
        attrs.canonicalise();
    }
}

/// What kind of checklist file a task lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// The owner's Inbox list, `tasks/<user>/inbox.md`.
    Inbox,
    /// Another personal or shared list under `tasks/`.
    List,
    /// A checklist inside an ordinary document.
    Doc,
}

/// The fixed status categories of an issue (brief section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueCategory {
    /// Waiting for triage.
    Triage,
    /// In the backlog.
    Backlog,
    /// Planned, not started.
    Unstarted,
    /// In progress.
    Started,
    /// Done.
    Completed,
    /// Cancelled.
    Canceled,
    /// Closed as a duplicate.
    Duplicate,
}

impl IssueCategory {
    /// Whether the issue is closed.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Duplicate)
    }
}

/// Where a task comes from. Only checklist items are stored as tasks;
/// the rest are joined in by the union query (D19).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// A checklist item: the Markdown block is the source of truth.
    Checklist {
        /// The document or list file.
        doc: DocId,
        /// The item's block ID.
        block: BlockId,
        /// What kind of file it is in.
        origin: Origin,
    },
    /// An issue assigned to the owner.
    Issue {
        /// The issue.
        issue: IssueId,
        /// Its key, such as `ENG-142`.
        key: String,
        /// Its status category.
        category: IssueCategory,
    },
    /// A comment that assigns something to the owner ("assign to").
    Comment {
        /// The comment.
        comment: CommentId,
        /// The document it is on.
        doc: DocId,
        /// Whether the comment thread is resolved.
        resolved: bool,
    },
    /// A review requested from the owner: a document review or a pull
    /// request linked to an issue.
    Review {
        /// A stable reference, such as `doc:<id>` or a pull request URL.
        reference: String,
        /// Whether the review was given.
        given: bool,
    },
}

impl Source {
    /// A stable key, unique across sources, for the calendar UID and for
    /// clients that need one.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Checklist { doc, block, .. } => format!("{doc}.{block}"),
            Self::Issue { issue, .. } => format!("issue.{issue}"),
            Self::Comment { comment, .. } => format!("comment.{comment}"),
            Self::Review { reference, .. } => format!("review.{reference}"),
        }
    }
}

/// A task as My Work shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Where it comes from.
    pub source: Source,
    /// The title, as plain text.
    pub title: String,
    /// Whether it is done (ticked, closed, resolved or given).
    pub done: bool,
    /// The structured fields.
    pub fields: TaskFields,
    /// User handles it is assigned to (`@handle`, without the `@`).
    pub assignees: Vec<String>,
    /// The parent checklist item, for subtasks.
    pub parent: Option<BlockId>,
    /// The list name (`#list` in quick-add), for checklist items in lists.
    pub list: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn due_round_trips() {
        for text in ["2026-11-03", "2026-11-03T15:00", "2026-11-03T15:00:30"] {
            assert_eq!(text.parse::<Due>().unwrap().to_string(), text);
        }
        assert!("tomorrow".parse::<Due>().is_err());
        let due: Due = "2026-11-03T15:00".parse().unwrap();
        assert_eq!(due.on(date(2026, 11, 10)).to_string(), "2026-11-10T15:00");
    }

    #[test]
    fn fields_round_trip_through_attrs() {
        let attrs = Attrs::parse(
            "{when=evening start=2026-11-01 due=2026-11-03T15:00 rrule=FREQ=WEEKLY \
             recur_from=completion remind=-PT15M,2026-11-03T09:00 p=1 labels=home,errand \
             done=2026-11-03T10:15:00Z src=todoist:1 extra=kept}",
        )
        .unwrap();
        let fields = TaskFields::from_attrs(&attrs).unwrap();
        assert_eq!(fields.when, Some(When::Evening));
        assert_eq!(fields.reminders.len(), 2);
        assert_eq!(fields.priority, Priority::new(1));
        let mut written = attrs;
        fields.write_attrs(&mut written);
        assert_eq!(
            written.to_string(),
            "{when=evening start=2026-11-01 due=2026-11-03T15:00 rrule=FREQ=WEEKLY \
             recur_from=completion remind=-PT15M,2026-11-03T09:00 p=1 labels=home,errand \
             done=2026-11-03T10:15:00Z src=todoist:1 extra=kept}"
        );
        assert_eq!(TaskFields::from_attrs(&written).unwrap(), fields);
    }

    #[test]
    fn invalid_values_are_reported() {
        for span in [
            "{p=7}",
            "{when=later}",
            "{due=soon}",
            "{remind=lunch}",
            "{rrule=FREQ=HOURLY}",
        ] {
            let attrs = Attrs::parse(span).unwrap();
            assert!(TaskFields::from_attrs(&attrs).is_err(), "{span}");
        }
    }
}
