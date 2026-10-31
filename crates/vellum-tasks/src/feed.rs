// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The per-user secret ICS feed (D20, R-MW-6).
//!
//! Google Calendar, Apple Calendar and Outlook subscribe to
//! `/feeds/tasks/<token>.ics`. The token is a kernel [`ShareToken`]: 22
//! base62 characters shown once, stored only as its SHA-256 hash,
//! revocable by rotating it. Calendar apps ignore `VTODO` in subscribed
//! feeds, so each dated open task is a `VEVENT` (D83): an all-day event on
//! its day, or a 30-minute event at its time, marked free so it never
//! blocks the owner's availability. Schedule-based repeats carry their
//! `RRULE`; completion-based repeats show only the next occurrence,
//! because the one after depends on when it is done.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan as _};
use vellum_kernel::{ShareToken, ShareTokenHash};

use crate::ical::{
    Component, Property, calendar, format_date, format_utc, rrule_value, uid,
};
use crate::model::{Due, RecurFrom, Task};
use crate::telemetry;

/// How long a timed task occupies on the calendar.
pub const EVENT_LENGTH: SignedDuration = SignedDuration::from_mins(30);

/// How far back completed-late and overdue tasks stay on the feed.
pub const LOOKBACK_DAYS: i64 = 30;

/// Settings for one feed.
#[derive(Debug, Clone)]
pub struct FeedOptions {
    /// The calendar name shown in the subscribing app.
    pub name: String,
    /// The owner's time zone.
    pub time_zone: TimeZone,
    /// The base URL of the Vellum web app, for links back.
    pub base_url: String,
    /// The owner's local date today.
    pub today: Date,
    /// When the feed is generated (`DTSTAMP`).
    pub now: Timestamp,
}

/// Issues a new feed token. Store [`ShareToken::hash`] and show the token
/// once, inside the feed URL; issuing a new one revokes the old.
///
/// # Errors
///
/// Fails only when the operating system's random source does.
pub fn issue_token()
-> Result<(ShareToken, ShareTokenHash), vellum_kernel::ShareTokenError> {
    let token = ShareToken::generate()?;
    let hash = token.hash();
    Ok((token, hash))
}

/// Reads the token from a feed path segment such as `<token>.ics`.
#[must_use]
pub fn token_from_path(segment: &str) -> Option<ShareToken> {
    segment.strip_suffix(".ics").unwrap_or(segment).parse().ok()
}

fn event(task: &Task, options: &FeedOptions) -> Option<Component> {
    let day = task
        .fields
        .due
        .or_else(|| task.fields.start.map(Due::Date))?;
    let cutoff = options.today.checked_sub(LOOKBACK_DAYS.days()).ok()?;
    let mut event = Component::new("VEVENT");
    event.push(Property::new("UID", uid(task)));
    event.push(Property::new("DTSTAMP", format_utc(options.now)));
    event.push_text("SUMMARY", &task.title);
    let timed = match day {
        Due::Date(date) => {
            let recurring = task
                .fields
                .recurrence
                .as_ref()
                .is_some_and(|r| r.from == RecurFrom::Schedule);
            if date < cutoff && !recurring {
                return None;
            }
            event.push(
                Property::new("DTSTART", format_date(date))
                    .param("VALUE", "DATE"),
            );
            let end = date.tomorrow().ok()?;
            event.push(
                Property::new("DTEND", format_date(end)).param("VALUE", "DATE"),
            );
            false
        }
        Due::DateTime(moment) => {
            if moment.date() < cutoff && task.fields.recurrence.is_none() {
                return None;
            }
            let start =
                moment.to_zoned(options.time_zone.clone()).ok()?.timestamp();
            event.push(Property::new("DTSTART", format_utc(start)));
            event.push(Property::new(
                "DTEND",
                format_utc(start.checked_add(EVENT_LENGTH).ok()?),
            ));
            true
        }
    };
    if let Some(recurrence) = task
        .fields
        .recurrence
        .as_ref()
        .filter(|r| r.from == RecurFrom::Schedule)
    {
        event
            .push(Property::new("RRULE", rrule_value(&recurrence.rule, timed)));
    }
    if !task.fields.labels.is_empty() {
        let labels: Vec<String> = task
            .fields
            .labels
            .iter()
            .map(|l| crate::ical::escape_text(l))
            .collect();
        event.push(Property::new("CATEGORIES", labels.join(",")));
    }
    let link = format!(
        "{}/my-work?task={}",
        options.base_url.trim_end_matches('/'),
        task.source.key()
    );
    event.push(Property::new("URL", link.clone()));
    event.push_text("DESCRIPTION", &format!("Open in Vellum: {link}"));
    event.push(Property::new("TRANSP", "TRANSPARENT"));
    event.push(Property::new("STATUS", "CONFIRMED"));
    Some(event)
}

/// Renders the feed for the owner's open tasks.
#[must_use]
pub fn render(tasks: &[Task], options: &FeedOptions) -> String {
    let _span =
        tracing::debug_span!("tasks.ics_feed", tasks = tasks.len()).entered();
    let events = tasks
        .iter()
        .filter(|t| !t.done)
        .filter_map(|t| event(t, options))
        .collect();
    let mut cal = calendar(Some(&options.name), events);
    cal.properties.insert(3, Property::new("METHOD", "PUBLISH"));
    cal.push(
        Property::new("REFRESH-INTERVAL", "PT15M").param("VALUE", "DURATION"),
    );
    cal.push(Property::new("X-PUBLISHED-TTL", "PT15M"));
    let text = cal.to_ical();
    telemetry::record_calendar("ics_feed", text.len());
    text
}

/// The `Content-Type` of the feed.
pub const CONTENT_TYPE: &str = "text/calendar; charset=utf-8";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attrs::Attrs;
    use crate::model::{Origin, Source, TaskFields};
    use jiff::civil::date;
    use vellum_kernel::{BlockId, DocId};

    fn task(title: &str, attrs: &str, done: bool) -> Task {
        Task {
            source: Source::Checklist {
                doc: DocId::generate(),
                block: BlockId::generate(),
                origin: Origin::List,
            },
            title: title.into(),
            done,
            fields: TaskFields::from_attrs(&Attrs::parse(attrs).unwrap())
                .unwrap(),
            assignees: vec![],
            parent: None,
            list: None,
        }
    }

    fn options() -> FeedOptions {
        FeedOptions {
            name: "My Work (Gourav)".into(),
            time_zone: TimeZone::get("Europe/London").unwrap(),
            base_url: "https://vellum.northstar.dev/".into(),
            today: date(2026, 11, 2),
            now: "2026-11-02T09:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn renders_dated_open_tasks_as_events() {
        let tasks = [
            task("All day", "{due=2026-11-03}", false),
            task("Timed", "{due=2026-07-03T15:00 rrule=FREQ=WEEKLY}", false),
            task("Starts", "{start=2026-11-05}", false),
            task("Undated", "{}", false),
            task("Done", "{due=2026-11-03}", true),
            task("Long ago", "{due=2026-01-01}", false),
            task(
                "From completion",
                "{due=2026-11-04 rrule=FREQ=DAILY recur_from=completion}",
                false,
            ),
        ];
        let text = render(&tasks, &options());
        assert!(text.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
        assert!(text.contains("METHOD:PUBLISH\r\n"));
        assert!(text.contains("X-WR-CALNAME:My Work (Gourav)\r\n"));
        assert_eq!(text.matches("BEGIN:VEVENT").count(), 4);
        assert!(text.contains(
            "DTSTART;VALUE=DATE:20261103\r\nDTEND;VALUE=DATE:20261104\r\n"
        ));
        // 15:00 London summer time is 14:00 UTC.
        assert!(text.contains(
            "DTSTART:20260703T140000Z\r\nDTEND:20260703T143000Z\r\n"
        ));
        assert!(text.contains("RRULE:FREQ=WEEKLY\r\n"));
        assert_eq!(text.matches("RRULE").count(), 1);
        assert!(
            text.contains("URL:https://vellum.northstar.dev/my-work?task=")
        );
        assert!(text.contains("TRANSP:TRANSPARENT"));
        assert!(!text.contains("Long ago") && !text.contains("SUMMARY:Done"));
    }

    #[test]
    fn tokens_are_issued_and_read_from_paths() {
        let (token, hash) = issue_token().unwrap();
        let path = format!("{}.ics", token.expose());
        let read = token_from_path(&path).unwrap();
        assert!(hash.verify(&read));
        assert!(token_from_path("not-a-token.ics").is_none());
    }
}
