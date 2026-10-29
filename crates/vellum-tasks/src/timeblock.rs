// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Time blocks and how they sync with Google Calendar (R-MW-9).
//!
//! A task gets a time block with `block=2026-11-03T15:00/90`: a local start
//! time and a length in minutes. When the person connects Google Calendar,
//! Vellum keeps one event per blocked task in a calendar of its own, called
//! "Vellum", and keeps both sides in step:
//!
//! - Moving or resizing the event in Google Calendar moves the block.
//! - Deleting the event clears the block; the task stays.
//! - A new timed event in that calendar becomes a task in the Inbox.
//! - Changing, completing or deleting the task updates or removes its event.
//!
//! The event's ID is kept on the task as `gcal=<id>`, and the event names
//! its task in a private extended property, so either side can find the
//! other. When both sides changed since the last sync, the calendar wins:
//! a moved event is the more deliberate edit of a time.
//!
//! [`plan`] is pure: it compares the tasks with the calendar's changes and
//! says what to do. `crate::google` carries the steps out.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;

use jiff::SignedDuration;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::FieldError;

/// The longest block, in minutes: one day.
pub const MAX_MINUTES: u32 = 24 * 60;

/// The private extended property that names an event's task.
pub const TASK_PROPERTY: &str = "vellumTask";

/// The name of the calendar Vellum creates for time blocks.
pub const CALENDAR_NAME: &str = "Vellum";

/// A planned stretch of time for a task, in the owner's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeBlock {
    /// When it starts.
    pub start: DateTime,
    /// How long it lasts, 1 to [`MAX_MINUTES`].
    pub minutes: u32,
}

impl TimeBlock {
    /// When it ends.
    #[must_use]
    pub fn end(&self) -> DateTime {
        self.start
            .checked_add(SignedDuration::from_mins(i64::from(self.minutes)))
            .unwrap_or(self.start)
    }
}

impl fmt::Display for TimeBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}",
            self.start.strftime("%Y-%m-%dT%H:%M"),
            self.minutes
        )
    }
}

impl FromStr for TimeBlock {
    type Err = FieldError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || FieldError::Value {
            field: "block",
            value: text.to_owned(),
        };
        let (start, minutes) = text.split_once('/').ok_or_else(bad)?;
        let start: DateTime = start.parse().map_err(|_| bad())?;
        let minutes: u32 = minutes.parse().map_err(|_| bad())?;
        if minutes == 0 || minutes > MAX_MINUTES {
            return Err(bad());
        }
        Ok(Self { start, minutes })
    }
}

/// A task as the calendar sync sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// The task ID, `<doc>.<block>`.
    pub task: String,
    /// Its title.
    pub title: String,
    /// Its time block, if it still has one.
    pub block: Option<TimeBlock>,
    /// Whether it is completed.
    pub done: bool,
    /// Its event, once it has one.
    pub event: Option<String>,
}

impl Blocked {
    /// What the event shows, so a change on either side is noticed.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let block = self.block.map(|b| b.to_string()).unwrap_or_default();
        format!("{block}|{}|{}", self.done, self.title)
    }

    /// The event's summary: the title, ticked when done.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.done {
            format!("✓ {}", self.title)
        } else {
            self.title.clone()
        }
    }
}

/// The Google Calendar event body for a blocked task. `zone` is the
/// owner's IANA time zone and `link` opens the task in Vellum.
#[must_use]
pub fn event_body(task: &Blocked, block: TimeBlock, zone: &str, link: &str) -> Value {
    let at = |time: DateTime| {
        json!({
            "dateTime": time.strftime("%Y-%m-%dT%H:%M:%S").to_string(),
            "timeZone": zone,
        })
    };
    json!({
        "summary": task.summary(),
        "description": format!("Vellum task: {link}"),
        "start": at(block.start),
        "end": at(block.end()),
        "transparency": "opaque",
        "extendedProperties": {"private": {TASK_PROPERTY: task.task}},
    })
}

/// An event as the sync reads it from Google Calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// The event ID.
    pub id: String,
    /// Whether it was deleted.
    pub cancelled: bool,
    /// Its title.
    pub summary: String,
    /// Its time, for timed events (all-day events have none).
    pub block: Option<TimeBlock>,
    /// The task it belongs to, from [`TASK_PROPERTY`].
    pub task: Option<String>,
}

fn instant(value: &Value, zone: &TimeZone) -> Option<DateTime> {
    let text = value.get("dateTime")?.as_str()?;
    let at: jiff::Timestamp = text.parse().ok()?;
    Some(at.to_zoned(zone.clone()).datetime())
}

/// Reads an event from the Calendar API's JSON, in the owner's time zone.
/// Returns `None` for something that is not an event.
#[must_use]
pub fn parse_event(value: &Value, zone: &TimeZone) -> Option<Remote> {
    let id = value.get("id")?.as_str()?.to_owned();
    let cancelled = value.get("status").and_then(Value::as_str) == Some("cancelled");
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim_start_matches("✓ ")
        .to_owned();
    let block = match (
        value.get("start").and_then(|s| instant(s, zone)),
        value.get("end").and_then(|e| instant(e, zone)),
    ) {
        (Some(start), Some(end)) => {
            let minutes = start.duration_until(end).as_mins();
            let minutes = u32::try_from(minutes.clamp(1, i64::from(MAX_MINUTES))).ok()?;
            Some(TimeBlock { start, minutes })
        }
        _ => None,
    };
    let task = value
        .pointer(&format!("/extendedProperties/private/{TASK_PROPERTY}"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(Remote {
        id,
        cancelled,
        summary,
        block,
        task,
    })
}

/// What the sync remembers between runs, per person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The calendar's incremental sync token.
    #[serde(default)]
    pub sync_token: Option<String>,
    /// For each event Vellum wrote, the fingerprint it last wrote.
    #[serde(default)]
    pub pushed: BTreeMap<String, String>,
}

/// One thing a sync does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The event moved in the calendar: give the task its time and link.
    Move {
        /// The task.
        task: String,
        /// Its event.
        event: String,
        /// The new block.
        block: TimeBlock,
    },
    /// The event was deleted or made all-day: clear the task's block and
    /// link.
    Unblock {
        /// The task.
        task: String,
        /// The event it was linked to, which Vellum stops tracking.
        event: Option<String>,
    },
    /// A new event in the Vellum calendar: add it to the Inbox.
    Adopt {
        /// The event.
        event: String,
        /// Its title.
        title: String,
        /// Its time.
        block: TimeBlock,
    },
    /// The task has no event yet: create one.
    Insert {
        /// The task.
        task: Blocked,
    },
    /// The task changed: update its event.
    Patch {
        /// The event.
        event: String,
        /// The task as it is now.
        task: Blocked,
    },
    /// The task is gone or unblocked: delete its event.
    Delete {
        /// The event.
        event: String,
    },
}

/// The tasks during planning, as the calendar's changes reshape them.
struct Planning<'a> {
    current: HashMap<&'a str, Blocked>,
    by_event: HashMap<&'a str, &'a str>,
    steps: Vec<Step>,
    settled: HashSet<String>,
}

impl<'a> Planning<'a> {
    fn new(tasks: &'a [Blocked]) -> Self {
        Self {
            current: tasks.iter().map(|t| (t.task.as_str(), t.clone())).collect(),
            by_event: tasks
                .iter()
                .filter_map(|t| t.event.as_deref().map(|e| (e, t.task.as_str())))
                .collect(),
            steps: Vec::new(),
            settled: HashSet::new(),
        }
    }

    /// One change from the calendar.
    fn pull(&mut self, remote: &Remote) {
        let linked = self.by_event.get(remote.id.as_str()).copied();
        let owner = remote.task.as_deref().or(linked);
        if remote.cancelled {
            if let Some(task) = linked {
                self.steps.push(Step::Unblock {
                    task: task.to_owned(),
                    event: Some(remote.id.clone()),
                });
                if let Some(t) = self.current.get_mut(task) {
                    t.block = None;
                    t.event = None;
                }
            }
            let _ = self.settled.insert(remote.id.clone());
            return;
        }
        let found = owner.and_then(|task| self.current.get_mut(task));
        match (found, remote.block) {
            (Some(task), Some(block)) => {
                if task.block != Some(block) || task.event.as_deref() != Some(&remote.id) {
                    self.steps.push(Step::Move {
                        task: task.task.clone(),
                        event: remote.id.clone(),
                        block,
                    });
                    task.block = Some(block);
                    task.event = Some(remote.id.clone());
                }
            }
            (Some(task), None) => {
                // Made all-day in the calendar: no longer a time block.
                self.steps.push(Step::Unblock {
                    task: task.task.clone(),
                    event: Some(remote.id.clone()),
                });
                task.block = None;
                task.event = None;
                let _ = self.settled.insert(remote.id.clone());
            }
            (None, _) if owner.is_some() => {
                // The task was deleted, or is no longer the person's.
                self.steps.push(Step::Delete {
                    event: remote.id.clone(),
                });
                let _ = self.settled.insert(remote.id.clone());
            }
            (None, block) => {
                if let Some(block) = block {
                    self.steps.push(Step::Adopt {
                        event: remote.id.clone(),
                        title: remote.summary.clone(),
                        block,
                    });
                }
                let _ = self.settled.insert(remote.id.clone());
            }
        }
    }

    /// The tasks' own changes, after the calendar's.
    fn push(mut self, state: &SyncState) -> Vec<Step> {
        let mut referenced: HashSet<String> = HashSet::new();
        let mut ordered: Vec<&Blocked> = self.current.values().collect();
        ordered.sort_by(|a, b| a.task.cmp(&b.task));
        for task in ordered {
            match (&task.event, task.block) {
                (None, Some(_)) => self.steps.push(Step::Insert { task: task.clone() }),
                (Some(event), None) => {
                    if !self.settled.contains(event) {
                        self.steps.push(Step::Delete {
                            event: event.clone(),
                        });
                    }
                    self.steps.push(Step::Unblock {
                        task: task.task.clone(),
                        event: Some(event.clone()),
                    });
                }
                (Some(event), Some(_)) => {
                    let _ = referenced.insert(event.clone());
                    if state.pushed.get(event) != Some(&task.fingerprint()) {
                        self.steps.push(Step::Patch {
                            event: event.clone(),
                            task: task.clone(),
                        });
                    }
                }
                (None, None) => {}
            }
        }
        for event in state.pushed.keys() {
            let known = referenced.contains(event)
                || self.settled.contains(event)
                || self.by_event.contains_key(event.as_str());
            if !known {
                self.steps.push(Step::Delete {
                    event: event.clone(),
                });
            }
        }
        self.steps
    }
}

/// Plans a sync: first the calendar's changes come in, then the tasks'
/// changes go out.
///
/// `tasks` are the person's tasks that have a block or an event; `changes`
/// are the events that changed since the last sync (or every event, on a
/// full sync).
#[must_use]
pub fn plan(tasks: &[Blocked], changes: &[Remote], state: &SyncState) -> Vec<Step> {
    let mut planning = Planning::new(tasks);
    for remote in changes {
        planning.pull(remote);
    }
    planning.push(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn block(hour: i8, minutes: u32) -> TimeBlock {
        TimeBlock {
            start: date(2026, 11, 3).at(hour, 0, 0, 0),
            minutes,
        }
    }

    fn task(id: &str, block: Option<TimeBlock>, event: Option<&str>) -> Blocked {
        Blocked {
            task: id.to_owned(),
            title: format!("Task {id}"),
            block,
            done: false,
            event: event.map(str::to_owned),
        }
    }

    fn remote(id: &str, task: Option<&str>, block: Option<TimeBlock>) -> Remote {
        Remote {
            id: id.to_owned(),
            cancelled: false,
            summary: "From the calendar".to_owned(),
            block,
            task: task.map(str::to_owned),
        }
    }

    fn pushed(pairs: &[(&str, &Blocked)]) -> SyncState {
        SyncState {
            sync_token: None,
            pushed: pairs
                .iter()
                .map(|(e, t)| ((*e).to_owned(), t.fingerprint()))
                .collect(),
        }
    }

    #[test]
    fn blocks_round_trip_and_reject_nonsense() {
        let b: TimeBlock = "2026-11-03T15:00/90".parse().unwrap();
        assert_eq!(b.to_string(), "2026-11-03T15:00/90");
        assert_eq!(b.end(), date(2026, 11, 3).at(16, 30, 0, 0));
        for bad in [
            "2026-11-03T15:00",
            "2026-11-03T15:00/0",
            "2026-11-03T15:00/1441",
            "x/30",
        ] {
            assert!(bad.parse::<TimeBlock>().is_err(), "{bad}");
        }
    }

    #[test]
    fn events_carry_the_task_and_read_back() {
        let t = task("d.b1", Some(block(15, 90)), None);
        let body = event_body(&t, block(15, 90), "Europe/Berlin", "https://x/t");
        assert_eq!(body["start"]["dateTime"], "2026-11-03T15:00:00");
        assert_eq!(body["end"]["dateTime"], "2026-11-03T16:30:00");
        let mut back = body;
        back["id"] = json!("e1");
        back["start"]["dateTime"] = json!("2026-11-03T15:00:00+01:00");
        back["end"]["dateTime"] = json!("2026-11-03T16:30:00+01:00");
        let zone = TimeZone::get("Europe/Berlin").unwrap();
        let read = parse_event(&back, &zone).unwrap();
        assert_eq!(read.task.as_deref(), Some("d.b1"));
        assert_eq!(read.block, Some(block(15, 90)));
        let all_day =
            json!({"id": "e2", "start": {"date": "2026-11-03"}, "end": {"date": "2026-11-04"}});
        assert_eq!(parse_event(&all_day, &zone).unwrap().block, None);
    }

    #[test]
    fn new_blocks_are_inserted_and_changes_patched() {
        let fresh = task("d.b1", Some(block(9, 30)), None);
        let synced = task("d.b2", Some(block(10, 30)), Some("e2"));
        let mut renamed = task("d.b3", Some(block(11, 30)), Some("e3"));
        let state = pushed(&[("e2", &synced), ("e3", &renamed)]);
        renamed.title = "Renamed".to_owned();
        let steps = plan(&[fresh.clone(), synced, renamed.clone()], &[], &state);
        assert_eq!(
            steps,
            vec![
                Step::Insert { task: fresh },
                Step::Patch {
                    event: "e3".to_owned(),
                    task: renamed
                },
            ]
        );
    }

    #[test]
    fn a_moved_event_moves_the_task_even_when_both_changed() {
        let mut local = task("d.b1", Some(block(9, 30)), Some("e1"));
        let state = pushed(&[("e1", &local)]);
        local.title = "Edited here too".to_owned();
        let steps = plan(
            &[local],
            &[remote("e1", Some("d.b1"), Some(block(14, 60)))],
            &state,
        );
        assert_eq!(
            steps.first(),
            Some(&Step::Move {
                task: "d.b1".to_owned(),
                event: "e1".to_owned(),
                block: block(14, 60)
            })
        );
        // The new title still goes out, at the calendar's time.
        assert!(
            matches!(steps.get(1), Some(Step::Patch { task, .. }) if task.block == Some(block(14, 60)))
        );
    }

    #[test]
    fn our_own_echo_changes_nothing() {
        let local = task("d.b1", Some(block(9, 30)), Some("e1"));
        let state = pushed(&[("e1", &local)]);
        let steps = plan(
            &[local],
            &[remote("e1", Some("d.b1"), Some(block(9, 30)))],
            &state,
        );
        assert!(steps.is_empty(), "{steps:?}");
    }

    #[test]
    fn deleting_either_side_unlinks_the_other() {
        let a = task("d.b1", Some(block(9, 30)), Some("e1"));
        let b = task("d.b2", None, Some("e2"));
        let state = pushed(&[("e1", &a), ("e2", &b), ("e3", &a)]);
        let mut cancelled = remote("e1", Some("d.b1"), None);
        cancelled.cancelled = true;
        let steps = plan(&[a, b], &[cancelled], &state);
        assert!(steps.contains(&Step::Unblock {
            task: "d.b1".to_owned(),
            event: Some("e1".to_owned())
        }));
        assert!(!steps.contains(&Step::Delete {
            event: "e1".to_owned()
        }));
        // Unblocked in Vellum: the event goes.
        assert!(steps.contains(&Step::Delete {
            event: "e2".to_owned()
        }));
        assert!(steps.contains(&Step::Unblock {
            task: "d.b2".to_owned(),
            event: Some("e2".to_owned())
        }));
        // The task behind e3 is gone.
        assert!(steps.contains(&Step::Delete {
            event: "e3".to_owned()
        }));
    }

    #[test]
    fn new_calendar_events_become_tasks() {
        let steps = plan(
            &[],
            &[remote("e9", None, Some(block(8, 45)))],
            &SyncState::default(),
        );
        assert_eq!(
            steps,
            vec![Step::Adopt {
                event: "e9".to_owned(),
                title: "From the calendar".to_owned(),
                block: block(8, 45)
            }]
        );
        assert!(plan(&[], &[remote("e9", None, None)], &SyncState::default()).is_empty());
    }

    #[test]
    fn an_event_for_a_vanished_task_is_deleted() {
        let steps = plan(
            &[],
            &[remote("e5", Some("d.gone"), Some(block(8, 45)))],
            &SyncState::default(),
        );
        assert_eq!(
            steps,
            vec![Step::Delete {
                event: "e5".to_owned()
            }]
        );
    }
}
