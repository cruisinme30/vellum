// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RFC 5545 iCalendar: content lines, components, and the mapping
//! between tasks and `VEVENT`/`VTODO` (D83).
//!
//! The ICS feed writes timed deadlines in UTC (computed in the owner's
//! zone) so every calendar app shows them at the right moment. CalDAV
//! `VTODO`s write them as floating local times, which is exactly how
//! tasks store them, so a round trip through Apple Reminders or
//! Thunderbird never shifts a deadline.

use std::fmt::Write as _;

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::model::{
    Due, Priority, RecurFrom, Recurrence, Reminder, Task, TaskFields,
};
use crate::recur::RRule;

/// One property: `NAME;PARAM=value:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Upper-case name.
    pub name: String,
    /// Parameters, names upper-case.
    pub params: Vec<(String, String)>,
    /// The raw value (still escaped for TEXT values).
    pub value: String,
}

impl Property {
    /// A property without parameters.
    #[must_use]
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            params: Vec::new(),
            value: value.into(),
        }
    }

    /// Adds a parameter.
    #[must_use]
    pub fn param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_owned(), value.to_owned()));
        self
    }

    /// A parameter's value.
    #[must_use]
    pub fn get_param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A component such as `VCALENDAR`, `VTODO` or `VALARM`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Component {
    /// Upper-case name.
    pub name: String,
    /// Its properties, in order.
    pub properties: Vec<Property>,
    /// Nested components.
    pub components: Vec<Self>,
}

/// Why iCalendar text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IcalError {
    /// A content line has no `:`.
    #[error("malformed content line {0}")]
    Line(usize),
    /// `BEGIN` and `END` do not match.
    #[error("unbalanced BEGIN/END for {0}")]
    Nesting(String),
    /// No component was found.
    #[error("no calendar component")]
    Empty,
    /// The calendar has no `VTODO`.
    #[error("no VTODO in the calendar object")]
    NoTodo,
    /// A date or date-time value is malformed.
    #[error("invalid date value {0:?}")]
    Date(String),
    /// The recurrence rule is not supported.
    #[error("unsupported recurrence: {0}")]
    RRule(String),
}

/// Escapes a TEXT value.
#[must_use]
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

/// Unescapes a TEXT value.
#[must_use]
pub fn unescape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn param_value(value: &str) -> String {
    if value.contains([':', ';', ',']) {
        format!("\"{}\"", value.replace('"', ""))
    } else {
        value.replace('"', "")
    }
}

/// Writes one content line, folded at 75 octets, ending in CRLF.
fn write_line(out: &mut String, line: &str) {
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

impl Component {
    /// An empty component.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// Adds a property.
    pub fn push(&mut self, property: Property) {
        self.properties.push(property);
    }

    /// Adds a TEXT property, escaping the value.
    pub fn push_text(&mut self, name: &str, text: &str) {
        self.push(Property::new(name, escape_text(text)));
    }

    /// The first property with this name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Every property with this name.
    pub fn all<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Property> {
        self.properties
            .iter()
            .filter(move |p| p.name.eq_ignore_ascii_case(name))
    }

    /// Writes the component as iCalendar text with CRLF line breaks.
    #[must_use]
    pub fn to_ical(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        write_line(out, &format!("BEGIN:{}", self.name));
        for property in &self.properties {
            let mut line = property.name.clone();
            for (name, value) in &property.params {
                let _ = write!(line, ";{name}={}", param_value(value));
            }
            line.push(':');
            line.push_str(&property.value);
            write_line(out, &line);
        }
        for component in &self.components {
            component.write(out);
        }
        write_line(out, &format!("END:{}", self.name));
    }

    /// Parses iCalendar text into its outermost component.
    ///
    /// # Errors
    ///
    /// Returns [`IcalError`] for malformed text.
    pub fn parse(text: &str) -> Result<Self, IcalError> {
        let mut unfolded: Vec<String> = Vec::new();
        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            if let Some(rest) = raw.strip_prefix([' ', '\t'])
                && let Some(last) = unfolded.last_mut()
            {
                last.push_str(rest);
                continue;
            }
            if !raw.is_empty() {
                unfolded.push(raw.to_owned());
            }
        }
        let mut stack: Vec<Self> = Vec::new();
        let mut root = None;
        for (index, line) in unfolded.iter().enumerate() {
            let property =
                parse_line(line).ok_or(IcalError::Line(index + 1))?;
            match property.name.as_str() {
                "BEGIN" => {
                    stack.push(Self::new(&property.value.to_ascii_uppercase()));
                }
                "END" => {
                    let done = stack
                        .pop()
                        .filter(|c| {
                            c.name.eq_ignore_ascii_case(&property.value)
                        })
                        .ok_or_else(|| {
                            IcalError::Nesting(property.value.clone())
                        })?;
                    if let Some(parent) = stack.last_mut() {
                        parent.components.push(done);
                    } else {
                        root = Some(done);
                        break;
                    }
                }
                _ => {
                    stack
                        .last_mut()
                        .ok_or(IcalError::Line(index + 1))?
                        .push(property);
                }
            }
        }
        if let Some(open) = stack.last() {
            return Err(IcalError::Nesting(open.name.clone()));
        }
        root.ok_or(IcalError::Empty)
    }
}

fn parse_line(line: &str) -> Option<Property> {
    // The name and parameters end at the first ':' outside quotes.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = line.get(..colon)?;
    let value = line.get(colon + 1..)?.to_owned();
    let mut parts = split_unquoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_owned(),
            ))
        })
        .collect();
    Some(Property {
        name,
        params,
        value,
    })
}

fn split_unquoted(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == sep && !in_quotes {
            out.push(text.get(start..i).unwrap_or_default());
            start = i + c.len_utf8();
        }
    }
    out.push(text.get(start..).unwrap_or_default());
    out
}

/// `YYYYMMDD`.
#[must_use]
pub fn format_date(date: Date) -> String {
    date.strftime("%Y%m%d").to_string()
}

/// `YYYYMMDDTHHMMSS` (floating).
#[must_use]
pub fn format_floating(moment: DateTime) -> String {
    moment.strftime("%Y%m%dT%H%M%S").to_string()
}

/// `YYYYMMDDTHHMMSSZ`.
#[must_use]
pub fn format_utc(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

/// A date or date-time property value, read in context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// A `DATE`.
    Date(Date),
    /// A `DATE-TIME`, converted to the owner's local time.
    Local(DateTime),
}

/// Reads a `DATE` or `DATE-TIME` property, converting UTC and `TZID`
/// values into the owner's zone. Floating values and unknown `TZID`s
/// are taken as the owner's local time.
///
/// # Errors
///
/// Returns [`IcalError::Date`] for a malformed value.
pub fn read_value(
    property: &Property,
    owner: &TimeZone,
) -> Result<Value, IcalError> {
    let raw = property.value.trim();
    let bad = || IcalError::Date(raw.to_owned());
    let date_of = |text: &str| -> Option<Date> {
        let y: i16 = text.get(..4)?.parse().ok()?;
        let m: i8 = text.get(4..6)?.parse().ok()?;
        let d: i8 = text.get(6..8)?.parse().ok()?;
        Date::new(y, m, d).ok()
    };
    if raw.len() == 8 || property.get_param("VALUE") == Some("DATE") {
        return date_of(raw).map(Value::Date).ok_or_else(bad);
    }
    let date = date_of(raw).ok_or_else(bad)?;
    let time_text = raw.get(9..15).ok_or_else(bad)?;
    let h: i8 = time_text
        .get(..2)
        .and_then(|t| t.parse().ok())
        .ok_or_else(bad)?;
    let m: i8 = time_text
        .get(2..4)
        .and_then(|t| t.parse().ok())
        .ok_or_else(bad)?;
    let s: i8 = time_text
        .get(4..6)
        .and_then(|t| t.parse().ok())
        .ok_or_else(bad)?;
    let local = date.at(h, m, s.min(59), 0);
    let zone = if raw.ends_with('Z') {
        Some(TimeZone::UTC)
    } else {
        property
            .get_param("TZID")
            .and_then(|z| TimeZone::get(z).ok())
    };
    match zone {
        Some(zone) => {
            let zoned = local.to_zoned(zone).map_err(|_| bad())?;
            Ok(Value::Local(zoned.with_time_zone(owner.clone()).datetime()))
        }
        None => Ok(Value::Local(local)),
    }
}

/// Maps a task priority to the iCalendar `PRIORITY` scale.
#[must_use]
pub const fn priority_to_ical(priority: Priority) -> u8 {
    match priority.level() {
        1 => 1,
        2 => 3,
        3 => 5,
        _ => 7,
    }
}

/// Maps an iCalendar `PRIORITY` (0 = none, 1 highest, 9 lowest).
#[must_use]
pub const fn priority_from_ical(value: u8) -> Option<Priority> {
    match value {
        1..=2 => Priority::new(1),
        3..=4 => Priority::new(2),
        5..=6 => Priority::new(3),
        7..=9 => Priority::new(4),
        _ => None,
    }
}

/// The rule as iCalendar wants it next to this start value: `UNTIL` must
/// be a UTC date-time when the start has a time.
#[must_use]
pub fn rrule_value(rule: &RRule, timed: bool) -> String {
    let text = rule.to_string();
    match (timed, rule.until) {
        (true, Some(until)) => text.replace(
            &format!("UNTIL={}", format_date(until)),
            &format!("UNTIL={}T235959Z", format_date(until)),
        ),
        _ => text,
    }
}

/// The prefix of `src` for tasks created by a CalDAV client, which keep
/// the client's UID.
pub const CALDAV_SRC: &str = "caldav:";

/// The calendar UID of a task: the client's own UID for tasks a CalDAV
/// client created, `<doc>.<block>@vellum` otherwise.
#[must_use]
pub fn uid(task: &Task) -> String {
    task.fields
        .src
        .as_deref()
        .and_then(|src| src.strip_prefix(CALDAV_SRC))
        .map_or_else(|| format!("{}@vellum", task.source.key()), str::to_owned)
}

fn duration_value(duration: SignedDuration) -> String {
    duration.to_string()
}

/// Builds the `VTODO` for a task (CalDAV, R-MW-6). `parent_uid` is the
/// UID of the parent task for subtasks; `owner` converts absolute
/// reminders to UTC.
#[must_use]
pub fn to_vtodo(
    task: &Task,
    parent_uid: Option<&str>,
    owner: &TimeZone,
    stamp: Timestamp,
) -> Component {
    let mut todo = Component::new("VTODO");
    todo.push(Property::new("UID", uid(task)));
    todo.push(Property::new("DTSTAMP", format_utc(stamp)));
    todo.push_text("SUMMARY", &task.title);
    let fields = &task.fields;
    if let Some(start) = fields.start {
        todo.push(
            Property::new("DTSTART", format_date(start)).param("VALUE", "DATE"),
        );
    }
    if let Some(due) = fields.due {
        todo.push(match due {
            Due::Date(day) => {
                Property::new("DUE", format_date(day)).param("VALUE", "DATE")
            }
            Due::DateTime(moment) => {
                Property::new("DUE", format_floating(moment))
            }
        });
    }
    if let Some(recurrence) = &fields.recurrence {
        let timed = matches!(fields.due, Some(Due::DateTime(_)));
        todo.push(Property::new("RRULE", rrule_value(&recurrence.rule, timed)));
        if recurrence.from == RecurFrom::Completion {
            todo.push(Property::new("X-VELLUM-RECUR-FROM", "completion"));
        }
    }
    if let Some(priority) = fields.priority {
        todo.push(Property::new(
            "PRIORITY",
            priority_to_ical(priority).to_string(),
        ));
    }
    if !fields.labels.is_empty() {
        let labels: Vec<String> =
            fields.labels.iter().map(|l| escape_text(l)).collect();
        todo.push(Property::new("CATEGORIES", labels.join(",")));
    }
    if task.done {
        todo.push(Property::new("STATUS", "COMPLETED"));
        todo.push(Property::new("PERCENT-COMPLETE", "100"));
        if let Some(done) = fields.done {
            todo.push(Property::new("COMPLETED", format_utc(done)));
        }
    } else {
        todo.push(Property::new("STATUS", "NEEDS-ACTION"));
    }
    if let Some(parent) = parent_uid {
        todo.push(
            Property::new("RELATED-TO", parent).param("RELTYPE", "PARENT"),
        );
    }
    for reminder in &fields.reminders {
        let mut alarm = Component::new("VALARM");
        alarm.push(Property::new("ACTION", "DISPLAY"));
        alarm.push_text("DESCRIPTION", &task.title);
        match reminder {
            Reminder::Relative(offset) => alarm.push(
                Property::new("TRIGGER", duration_value(*offset))
                    .param("RELATED", "END"),
            ),
            Reminder::At(moment) => {
                if let Ok(zoned) = moment.to_zoned(owner.clone()) {
                    alarm.push(
                        Property::new("TRIGGER", format_utc(zoned.timestamp()))
                            .param("VALUE", "DATE-TIME"),
                    );
                }
            }
        }
        todo.components.push(alarm);
    }
    todo
}

/// Wraps components in a `VCALENDAR`.
#[must_use]
pub fn calendar(name: Option<&str>, components: Vec<Component>) -> Component {
    let mut cal = Component::new("VCALENDAR");
    cal.push(Property::new("VERSION", "2.0"));
    cal.push(Property::new("PRODID", "-//Vellum//My Work//EN"));
    cal.push(Property::new("CALSCALE", "GREGORIAN"));
    if let Some(name) = name {
        cal.push_text("X-WR-CALNAME", name);
    }
    cal.components = components;
    cal
}

/// A task as a CalDAV client sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoUpdate {
    /// The UID.
    pub uid: String,
    /// The title.
    pub title: String,
    /// Whether it is completed.
    pub done: bool,
    /// The fields.
    pub fields: TaskFields,
    /// The parent's UID, for subtasks.
    pub parent_uid: Option<String>,
}

/// Reads the `VTODO` of a CalDAV object. `now` is the completion time
/// when the client marked it done without a `COMPLETED` value.
///
/// # Errors
///
/// Returns [`IcalError`] for malformed text, a missing `VTODO` or an
/// unsupported rule.
pub fn from_vtodo(
    text: &str,
    owner: &TimeZone,
    now: Timestamp,
) -> Result<TodoUpdate, IcalError> {
    let root = Component::parse(text)?;
    let todo = if root.name == "VTODO" {
        &root
    } else {
        root.components
            .iter()
            .find(|c| c.name == "VTODO")
            .ok_or(IcalError::NoTodo)?
    };
    let text_of = |name: &str| todo.get(name).map(|p| unescape_text(&p.value));
    let mut fields = TaskFields::default();
    if let Some(property) = todo.get("DTSTART") {
        fields.start = Some(match read_value(property, owner)? {
            Value::Date(day) => day,
            Value::Local(moment) => moment.date(),
        });
    }
    if let Some(property) = todo.get("DUE") {
        fields.due = Some(match read_value(property, owner)? {
            Value::Date(day) => Due::Date(day),
            Value::Local(moment) => Due::DateTime(moment),
        });
    }
    if let Some(property) = todo.get("RRULE") {
        let rule: RRule =
            property
                .value
                .parse()
                .map_err(|e: crate::recur::RRuleError| {
                    IcalError::RRule(e.to_string())
                })?;
        let from = if text_of("X-VELLUM-RECUR-FROM").as_deref()
            == Some("completion")
        {
            RecurFrom::Completion
        } else {
            RecurFrom::Schedule
        };
        fields.recurrence = Some(Recurrence { rule, from });
    }
    fields.priority = todo
        .get("PRIORITY")
        .and_then(|p| p.value.trim().parse::<u8>().ok())
        .and_then(priority_from_ical);
    fields.labels = todo
        .all("CATEGORIES")
        .flat_map(|p| {
            split_unquoted(&p.value, ',')
                .into_iter()
                .map(unescape_text)
                .collect::<Vec<_>>()
        })
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    let status = text_of("STATUS").unwrap_or_default().to_ascii_uppercase();
    let completed = todo.get("COMPLETED");
    let done = status == "COMPLETED"
        || (completed.is_some() && status != "NEEDS-ACTION");
    if done {
        fields.done = Some(
            completed
                .and_then(|p| read_value(p, &TimeZone::UTC).ok())
                .and_then(|v| match v {
                    Value::Local(moment) => moment.to_zoned(TimeZone::UTC).ok(),
                    Value::Date(day) => day.to_zoned(TimeZone::UTC).ok(),
                })
                .map_or(now, |z| z.timestamp()),
        );
    }
    for alarm in todo.components.iter().filter(|c| c.name == "VALARM") {
        let Some(trigger) = alarm.get("TRIGGER") else {
            continue;
        };
        let reminder = if trigger.get_param("VALUE") == Some("DATE-TIME") {
            match read_value(trigger, owner)? {
                Value::Local(moment) => Some(Reminder::At(moment)),
                Value::Date(day) => Some(Reminder::At(
                    day.to_datetime(jiff::civil::Time::midnight()),
                )),
            }
        } else {
            trigger.value.parse::<Reminder>().ok()
        };
        fields.reminders.extend(reminder);
    }
    let parent_uid = todo
        .all("RELATED-TO")
        .find(|p| {
            p.get_param("RELTYPE")
                .is_none_or(|t| t.eq_ignore_ascii_case("PARENT"))
        })
        .map(|p| p.value.trim().to_owned());
    Ok(TodoUpdate {
        uid: text_of("UID").unwrap_or_default().trim().to_owned(),
        title: text_of("SUMMARY")
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        done,
        fields,
        parent_uid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, Source};
    use jiff::civil::date;
    use vellum_kernel::{BlockId, DocId};

    fn task() -> Task {
        let attrs = crate::attrs::Attrs::parse(
            "{start=2026-11-01 due=2026-11-03T15:00 \
             rrule=FREQ=WEEKLY;UNTIL=20261231 recur_from=completion \
             remind=-PT15M,2026-11-03T08:00 p=2 labels=home,a;b}",
        )
        .unwrap();
        Task {
            source: Source::Checklist {
                doc: "01M639NFH15EBMG6C7NX1ZK4KA".parse::<DocId>().unwrap(),
                block: "blk-7f3a9k2m".parse::<BlockId>().unwrap(),
                origin: Origin::List,
            },
            title: "Send the Northstar invoice, v2; final".into(),
            done: false,
            fields: TaskFields::from_attrs(&attrs).unwrap(),
            assignees: vec![],
            parent: None,
            list: None,
        }
    }

    #[test]
    fn text_escaping_round_trips() {
        let text = "a, b; c\\d\nline";
        assert_eq!(escape_text(text), "a\\, b\\; c\\\\d\\nline");
        assert_eq!(unescape_text(&escape_text(text)), text);
    }

    #[test]
    fn long_lines_fold_and_unfold() {
        let mut c = Component::new("VTODO");
        c.push_text("SUMMARY", &"é".repeat(100));
        let text = c.to_ical();
        assert!(text.lines().all(|l| l.trim_end_matches('\r').len() <= 75));
        assert_eq!(Component::parse(&text).unwrap(), c);
    }

    #[test]
    fn vtodo_round_trips_through_a_client() {
        let zone = TimeZone::get("Europe/London").unwrap();
        let stamp: Timestamp = "2026-11-01T00:00:00Z".parse().unwrap();
        let t = task();
        let cal = calendar(
            Some("Errands"),
            vec![to_vtodo(&t, Some("parent@vellum"), &zone, stamp)],
        );
        let text = cal.to_ical();
        assert!(text.contains("DUE:20261103T150000\r\n"));
        assert!(text.contains("RRULE:FREQ=WEEKLY;UNTIL=20261231T235959Z\r\n"));
        assert!(
            text.contains(
                "SUMMARY:Send the Northstar invoice\\, v2\\; final\r\n"
            )
        );
        assert!(text.contains("TRIGGER;RELATED=END:-PT15M\r\n"));
        assert!(text.contains("TRIGGER;VALUE=DATE-TIME:20261103T080000Z\r\n"));
        let back = from_vtodo(&text, &zone, stamp).unwrap();
        assert_eq!(back.uid, uid(&t));
        assert_eq!(back.title, t.title);
        assert_eq!(back.parent_uid.as_deref(), Some("parent@vellum"));
        assert!(!back.done);
        assert_eq!(back.fields.due, t.fields.due);
        assert_eq!(back.fields.start, t.fields.start);
        assert_eq!(back.fields.priority, t.fields.priority);
        assert_eq!(back.fields.labels, t.fields.labels);
        assert_eq!(back.fields.reminders, t.fields.reminders);
        let rec = back.fields.recurrence.unwrap();
        assert_eq!(rec.from, RecurFrom::Completion);
        assert_eq!(rec.rule.until, Some(date(2026, 12, 31)));
    }

    #[test]
    fn client_values_are_converted_to_the_owner_zone() {
        let zone = TimeZone::get("America/New_York").unwrap();
        let now: Timestamp = "2026-11-05T12:00:00Z".parse().unwrap();
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:x\r\nSUMMARY:Call\r\n\
DUE;TZID=Europe/Berlin:20261103T150000\r\nDTSTART:20261102T230000Z\r\n\
STATUS:COMPLETED\r\nPRIORITY:9\r\nCATEGORIES:a,b\r\nCATEGORIES:c\r\n\
END:VTODO\r\nEND:VCALENDAR\r\n";
        let todo = from_vtodo(text, &zone, now).unwrap();
        assert_eq!(todo.fields.due.unwrap().to_string(), "2026-11-03T09:00");
        assert_eq!(todo.fields.start, Some(date(2026, 11, 2)));
        assert!(todo.done);
        assert_eq!(todo.fields.done, Some(now));
        assert_eq!(todo.fields.priority, Priority::new(4));
        assert_eq!(todo.fields.labels, ["a", "b", "c"]);
    }

    #[test]
    fn malformed_input_is_refused() {
        let zone = TimeZone::UTC;
        let now = Timestamp::UNIX_EPOCH;
        assert_eq!(Component::parse(""), Err(IcalError::Empty));
        assert!(matches!(
            Component::parse("BEGIN:VTODO\r\nEND:VEVENT\r\n"),
            Err(IcalError::Nesting(_))
        ));
        assert!(matches!(
            Component::parse("BEGIN:VTODO\r\nnonsense\r\n"),
            Err(IcalError::Line(2))
        ));
        assert_eq!(
            from_vtodo("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", &zone, now),
            Err(IcalError::NoTodo)
        );
        let bad_rule =
            "BEGIN:VTODO\r\nUID:x\r\nRRULE:FREQ=HOURLY\r\nEND:VTODO\r\n";
        assert!(matches!(
            from_vtodo(bad_rule, &zone, now),
            Err(IcalError::RRule(_))
        ));
        let bad_date = "BEGIN:VTODO\r\nUID:x\r\nDUE:2026\r\nEND:VTODO\r\n";
        assert!(matches!(
            from_vtodo(bad_date, &zone, now),
            Err(IcalError::Date(_))
        ));
    }

    #[test]
    fn priorities_map_both_ways() {
        for level in 1..=4 {
            let p = Priority::new(level).unwrap();
            assert_eq!(priority_from_ical(priority_to_ical(p)), Some(p));
        }
        assert_eq!(priority_from_ical(0), None);
    }
}
