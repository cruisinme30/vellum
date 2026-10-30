// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reminders and the daily digest (R-MW-5).
//!
//! This module decides *when*: which reminders fall due in a window and
//! what the morning digest says. Delivery (web push, desktop
//! notifications, e-mail) belongs to the notifications service, which
//! calls [`due_between`] on a timer and sends what it returns.

use std::fmt::Write as _;

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use crate::model::{Due, Reminder, Source, Task};
use crate::projection::{Entry, MyWork, UpcomingGroup};
use crate::telemetry;

/// A person's reminder settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Their time zone.
    pub time_zone: TimeZone,
    /// The time a relative reminder on a date-only deadline counts from.
    pub day_starts_at: Time,
    /// When the daily digest is sent; `None` turns it off.
    pub digest_at: Option<Time>,
}

impl Settings {
    /// Defaults: the day starts at 09:00 and the digest goes at 07:30.
    #[must_use]
    pub const fn new(time_zone: TimeZone) -> Self {
        Self {
            time_zone,
            day_starts_at: Time::constant(9, 0, 0, 0),
            digest_at: Some(Time::constant(7, 30, 0, 0)),
        }
    }
}

/// A reminder that has fallen due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueReminder {
    /// The task's stable key ([`Source::key`]).
    pub task: String,
    /// The task title, for the notification.
    pub title: String,
    /// When it fires.
    pub at: Timestamp,
}

/// The instants a task's reminders fire at.
#[must_use]
pub fn instants(task: &Task, settings: &Settings) -> Vec<Timestamp> {
    let anchor = task.fields.due.map(|due| match due {
        Due::Date(day) => day.to_datetime(settings.day_starts_at),
        Due::DateTime(moment) => moment,
    });
    task.fields
        .reminders
        .iter()
        .filter_map(|reminder| {
            let local = match reminder {
                Reminder::At(moment) => *moment,
                Reminder::Relative(offset) => {
                    anchor?.checked_add(*offset).ok()?
                }
            };
            local
                .to_zoned(settings.time_zone.clone())
                .ok()
                .map(|z| z.timestamp())
        })
        .collect()
}

/// Reminders of open tasks that fire in `(after, until]`, in time order.
#[must_use]
pub fn due_between(
    tasks: &[Task],
    after: Timestamp,
    until: Timestamp,
    settings: &Settings,
) -> Vec<DueReminder> {
    let _span =
        tracing::debug_span!("tasks.reminders_due", tasks = tasks.len())
            .entered();
    let mut out: Vec<DueReminder> = tasks
        .iter()
        .filter(|t| !t.done)
        .flat_map(|task| {
            instants(task, settings)
                .into_iter()
                .filter(|at| *at > after && *at <= until)
                .map(|at| DueReminder {
                    task: task.source.key(),
                    title: task.title.clone(),
                    at,
                })
        })
        .collect();
    out.sort_by_key(|r| r.at);
    telemetry::record_reminders(out.len());
    out
}

/// The next time the digest is due strictly after `now`, or `None` when
/// the digest is off.
#[must_use]
pub fn next_digest(now: &Zoned, settings: &Settings) -> Option<Zoned> {
    let at = settings.digest_at?;
    let local = now.with_time_zone(settings.time_zone.clone());
    let today = local
        .date()
        .to_datetime(at)
        .to_zoned(settings.time_zone.clone())
        .ok()?;
    if today.timestamp() > now.timestamp() {
        return Some(today);
    }
    local
        .date()
        .tomorrow()
        .ok()?
        .to_datetime(at)
        .to_zoned(settings.time_zone.clone())
        .ok()
}

fn link(task: &Task) -> String {
    let title = task.title.replace(['[', ']'], "");
    match &task.source {
        Source::Checklist { doc, .. } | Source::Comment { doc, .. } => {
            format!("[{title}](vellum://doc/{doc})")
        }
        Source::Mention { doc: Some(doc), .. } => {
            format!("[{title}](vellum://doc/{doc})")
        }
        Source::Issue { key, .. }
        | Source::Mention {
            issue: Some(key), ..
        } => format!("{key} {title}"),
        Source::Mention { .. } => title,
        Source::Review { .. } => format!("Review: {title}"),
    }
}

fn short_date(date: Date) -> String {
    date.strftime("%a %-d %b").to_string()
}

fn line(out: &mut String, entry: &Entry, today: Date) {
    let mut text = link(&entry.task);
    if let Some(due) = entry.task.fields.due {
        if entry.overdue {
            let _ = write!(text, " (due {})", short_date(due.date()));
        } else if let Some(moment) =
            due.datetime().filter(|m| m.date() == today)
        {
            let _ = write!(text, " ({})", moment.strftime("%H:%M"));
        }
    }
    let _ = writeln!(out, "- {text}");
}

/// The daily digest as Vellum Markdown, or `None` when there is nothing
/// to do today, nothing overdue and nothing in the Inbox.
#[must_use]
pub fn digest(view: &MyWork, today: Date) -> Option<String> {
    let overdue: Vec<&Entry> =
        view.today.iter().filter(|e| e.overdue).collect();
    let today_only: Vec<&Entry> =
        view.today.iter().filter(|e| !e.overdue).collect();
    if view.today.is_empty() && view.evening.is_empty() && view.inbox.is_empty()
    {
        return None;
    }
    let mut out = String::new();
    let _ = writeln!(out, "# My Work for {}\n", today.strftime("%A %-d %B"));
    let week: usize = view
        .upcoming
        .iter()
        .filter(|(group, _)| matches!(group, UpcomingGroup::Day(_)))
        .map(|(_, entries)| entries.len())
        .sum();
    let _ = writeln!(
        out,
        "{} today, {} overdue, {} this evening, {} in the next seven days, {} \
            in the Inbox.\n",
        today_only.len(),
        overdue.len(),
        view.evening.len(),
        week,
        view.inbox.len()
    );
    let section = |out: &mut String, heading: &str, entries: &[&Entry]| {
        if entries.is_empty() {
            return;
        }
        let _ = writeln!(out, "## {heading}\n");
        for entry in entries {
            line(out, entry, today);
        }
        out.push('\n');
    };
    section(&mut out, "Overdue", &overdue);
    section(&mut out, "Today", &today_only);
    section(
        &mut out,
        "This evening",
        &view.evening.iter().collect::<Vec<_>>(),
    );
    section(&mut out, "Inbox", &view.inbox.iter().collect::<Vec<_>>());
    Some(out.trim_end().to_owned() + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TaskFields;
    use crate::projection::build;
    use jiff::civil::date;
    use vellum_kernel::{BlockId, DocId};

    fn task(title: &str, attrs: &str) -> Task {
        let attrs = crate::attrs::Attrs::parse(attrs).unwrap();
        Task {
            source: Source::Checklist {
                doc: DocId::generate(),
                block: BlockId::generate(),
                origin: crate::model::Origin::List,
            },
            title: title.into(),
            done: false,
            fields: TaskFields::from_attrs(&attrs).unwrap(),
            assignees: vec![],
            parent: None,
            list: None,
        }
    }

    fn settings() -> Settings {
        Settings::new(TimeZone::get("Europe/London").unwrap())
    }

    #[test]
    fn reminder_instants_follow_the_time_zone() {
        let t = task(
            "Call",
            "{due=2026-11-03T15:00 remind=-PT15M,2026-11-03T08:00}",
        );
        let at: Vec<String> = instants(&t, &settings())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(at, ["2026-11-03T14:45:00Z", "2026-11-03T08:00:00Z"]);
        // Summer time: London is UTC+1.
        let t = task("Call", "{due=2026-07-03 remind=-PT1H}");
        let at: Vec<String> = instants(&t, &settings())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(at, ["2026-07-03T07:00:00Z"]);
        // A relative reminder needs a deadline.
        assert!(instants(&task("x", "{remind=-PT1H}"), &settings()).is_empty());
    }

    #[test]
    fn due_between_is_half_open_and_sorted() {
        let tasks = vec![
            task("Late", "{due=2026-11-03T15:00 remind=-PT15M}"),
            task("Early", "{remind=2026-11-03T08:00}"),
        ];
        let after: Timestamp = "2026-11-03T08:00:00Z".parse().unwrap();
        let until: Timestamp = "2026-11-03T15:00:00Z".parse().unwrap();
        let due = due_between(&tasks, after, until, &settings());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].title, "Late");
        let earlier: Timestamp = "2026-11-03T07:00:00Z".parse().unwrap();
        let due = due_between(&tasks, earlier, until, &settings());
        assert_eq!(
            due.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            ["Early", "Late"]
        );
    }

    #[test]
    fn next_digest_is_today_or_tomorrow() {
        let s = settings();
        let before: Zoned = "2026-11-03T06:00[Europe/London]".parse().unwrap();
        assert_eq!(
            next_digest(&before, &s).unwrap().to_string(),
            "2026-11-03T07:30:00+00:00[Europe/London]"
        );
        let after: Zoned = "2026-11-03T07:30[Europe/London]".parse().unwrap();
        assert_eq!(
            next_digest(&after, &s).unwrap().to_string(),
            "2026-11-04T07:30:00+00:00[Europe/London]"
        );
        let off = Settings {
            digest_at: None,
            ..s
        };
        assert_eq!(next_digest(&after, &off), None);
    }

    #[test]
    fn digest_lists_overdue_today_evening_and_inbox() {
        let today = date(2026, 11, 3);
        let view = build(
            vec![
                task("Pay invoice", "{due=2026-11-01}"),
                task("Call Ana", "{due=2026-11-03T15:00}"),
                task("Read", "{when=evening start=2026-11-03}"),
                task("Plan", "{start=2026-11-05}"),
            ],
            today,
        );
        let text = digest(&view, today).unwrap();
        assert!(text.starts_with(
            "# My Work for Tuesday 3 November\n\n\
            1 today, 1 overdue, 1 this evening, 1 in the next seven days, 0 in \
            the Inbox.\n"
        ));
        assert!(text.contains("## Overdue\n\n- [Pay invoice](vellum://doc/"));
        assert!(text.contains(") (due Sun 1 Nov)\n"));
        assert!(text.contains(") (15:00)\n"));
        assert!(text.contains("## This evening\n"));
        assert!(!text.contains("## Inbox"));
        assert_eq!(digest(&MyWork::default(), today), None);
    }
}
