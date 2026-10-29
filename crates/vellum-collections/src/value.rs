// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Cell values.
//!
//! Every property value, stored or computed, is a [`Value`]. Values are
//! dynamically typed so that formulas and rollups can mix them; the schema
//! says which shape each stored property must have.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::{Date, Month, Time};

/// A date with an optional time of day, always in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateValue {
    date: Date,
    time: Option<Time>,
}

impl DateValue {
    /// A whole-day date.
    #[must_use]
    pub const fn day(date: Date) -> Self {
        Self { date, time: None }
    }

    /// A date with a time of day.
    #[must_use]
    pub const fn at(date: Date, time: Time) -> Self {
        Self {
            date,
            time: Some(time),
        }
    }

    /// Builds a whole-day date from its parts, if they are valid.
    #[must_use]
    pub fn ymd(year: i32, month: u8, day: u8) -> Option<Self> {
        let month = Month::try_from(month).ok()?;
        Date::from_calendar_date(year, month, day)
            .ok()
            .map(Self::day)
    }

    /// The date from Unix milliseconds (UTC), with its time of day.
    #[must_use]
    pub fn from_unix_ms(ms: i64) -> Option<Self> {
        let nanos = i128::from(ms).checked_mul(1_000_000)?;
        let at = time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
        let time = at.time().replace_millisecond(at.millisecond()).ok()?;
        Some(Self::at(at.date(), time))
    }

    /// The calendar date.
    #[must_use]
    pub const fn date(self) -> Date {
        self.date
    }

    /// The time of day, if one was given.
    #[must_use]
    pub const fn time(self) -> Option<Time> {
        self.time
    }

    /// Parses `YYYY-MM-DD`, optionally followed by `THH:MM[:SS]` and `Z`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (day, rest) = match text.split_once(['T', ' ']) {
            Some((day, rest)) => (day, Some(rest.trim_end_matches('Z'))),
            None => (text, None),
        };
        let mut parts = day.splitn(3, '-');
        let year: i32 = parse_digits(parts.next()?, 4)?;
        let month: u8 = parse_digits(parts.next()?, 2)?;
        let dom: u8 = parse_digits(parts.next()?, 2)?;
        let date = Self::ymd(year, month, dom)?.date;
        let Some(rest) = rest else {
            return Some(Self::day(date));
        };
        let mut parts = rest.splitn(3, ':');
        let hour: u8 = parse_digits(parts.next()?, 2)?;
        let minute: u8 = parse_digits(parts.next()?, 2)?;
        let second: u8 = match parts.next() {
            Some(s) => parse_digits(s, 2)?,
            None => 0,
        };
        let time = Time::from_hms(hour, minute, second).ok()?;
        Some(Self::at(date, time))
    }

    /// Whole days from `other` to `self` (`self - other`), ignoring times.
    #[must_use]
    pub fn days_since(self, other: Self) -> i64 {
        (self.date - other.date).whole_days()
    }

    /// This date moved by a whole number of days, if it stays in range.
    #[must_use]
    pub fn add_days(self, days: i64) -> Option<Self> {
        let date = self.date.checked_add(time::Duration::days(days))?;
        Some(Self {
            date,
            time: self.time,
        })
    }

    fn sort_key(self) -> (Date, Time) {
        (self.date, self.time.unwrap_or(Time::MIDNIGHT))
    }
}

fn parse_digits<T: std::str::FromStr>(text: &str, len: usize) -> Option<T> {
    if text.len() == len && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

impl PartialOrd for DateValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl fmt::Display for DateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.date;
        write!(
            f,
            "{:04}-{:02}-{:02}",
            d.year(),
            u8::from(d.month()),
            d.day()
        )?;
        if let Some(t) = self.time {
            write!(f, "T{:02}:{:02}", t.hour(), t.minute())?;
            if t.second() != 0 {
                write!(f, ":{:02}", t.second())?;
            }
        }
        Ok(())
    }
}

impl schemars::JsonSchema for DateValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DateValue".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A UTC date, YYYY-MM-DD, optionally with THH:MM[:SS]",
            "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}(T[0-9]{2}:[0-9]{2}(:[0-9]{2})?)?$",
        })
    }
}

impl Serialize for DateValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DateValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom("expected YYYY-MM-DD[THH:MM]"))
    }
}

/// A property value.
///
/// Serialised for the API as `{"type": "number", "value": 3}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// No value.
    #[default]
    Empty,
    /// Text, a select option, a URL, a person handle or a row ID.
    Text(String),
    /// A finite number.
    Number(f64),
    /// A checkbox or a comparison result.
    Bool(bool),
    /// A date, with or without a time.
    Date(DateValue),
    /// Several values: multi-select options, related rows, rollup `show`.
    List(Vec<Self>),
}

impl Value {
    /// Text from anything string-like.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// Whether the value counts as empty: nothing, blank text or an empty list.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(t) => t.is_empty(),
            Self::List(items) => items.iter().all(Self::is_empty),
            Self::Number(_) | Self::Bool(_) | Self::Date(_) => false,
        }
    }

    /// Truthiness for `if`, `and`, `or` and `not`.
    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0,
            Self::Date(_) => true,
            Self::Empty | Self::Text(_) | Self::List(_) => !self.is_empty(),
        }
    }

    /// The value as a number, if it is one.
    #[must_use]
    pub const fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The value as a date, if it is one.
    #[must_use]
    pub const fn as_date(&self) -> Option<DateValue> {
        match self {
            Self::Date(d) => Some(*d),
            _ => None,
        }
    }

    /// The value as text, if it is text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    /// The items of a list, or the value itself as a one-item slice.
    #[must_use]
    pub fn items(&self) -> &[Self] {
        match self {
            Self::List(items) => items,
            Self::Empty => &[],
            other => std::slice::from_ref(other),
        }
    }

    /// Flattens nested lists into their non-empty leaf values.
    #[must_use]
    pub fn leaves(&self) -> Vec<&Self> {
        let mut out = Vec::new();
        collect_leaves(self, &mut out);
        out
    }

    /// The type name used in error messages.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Text(_) => "text",
            Self::Number(_) => "number",
            Self::Bool(_) => "checkbox",
            Self::Date(_) => "date",
            Self::List(_) => "list",
        }
    }

    /// Total order used by sorts and group-by: empty, booleans, numbers,
    /// dates, text (case-insensitive), lists.
    #[must_use]
    pub fn sort_cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Date(a), Self::Date(b)) => a.cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Text(a), Self::Text(b)) => a
                .to_lowercase()
                .cmp(&b.to_lowercase())
                .then_with(|| a.cmp(b)),
            (Self::List(a), Self::List(b)) => {
                for (x, y) in a.iter().zip(b) {
                    let ord = x.sort_cmp(y);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                a.len().cmp(&b.len())
            }
            _ => self.rank().cmp(&other.rank()),
        }
    }

    const fn rank(&self) -> u8 {
        match self {
            Self::Empty => 0,
            Self::Bool(_) => 1,
            Self::Number(_) => 2,
            Self::Date(_) => 3,
            Self::Text(_) => 4,
            Self::List(_) => 5,
        }
    }
}

fn collect_leaves<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::List(items) => items.iter().for_each(|v| collect_leaves(v, out)),
        v if v.is_empty() => {}
        v => out.push(v),
    }
}

/// Formats a number without float noise: `5`, `1.5`, `0.3` (not
/// `0.30000000000000004`).
#[must_use]
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        // The value is integral and well inside i64, so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        let whole = n as i64;
        return whole.to_string();
    }
    let rounded = (n * 1e10).round() / 1e10;
    if rounded.is_finite() && rounded != 0.0 {
        rounded.to_string()
    } else {
        n.to_string()
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => Ok(()),
            Self::Text(t) => f.write_str(t),
            Self::Number(n) => f.write_str(&format_number(*n)),
            Self::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Self::Date(d) => d.fmt(f),
            Self::List(items) => {
                let mut first = true;
                for item in items.iter().filter(|v| !v.is_empty()) {
                    if !first {
                        f.write_str(", ")?;
                    }
                    first = false;
                    item.fmt(f)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse_and_print() {
        let d = DateValue::parse("2026-10-28").unwrap_or_else(|| unreachable!());
        assert_eq!(d.to_string(), "2026-10-28");
        let t = DateValue::parse("2026-10-28T09:30Z").unwrap_or_else(|| unreachable!());
        assert_eq!(t.to_string(), "2026-10-28T09:30");
        assert!(DateValue::parse("2026-13-01").is_none());
        assert!(DateValue::parse("26-1-1").is_none());
        assert!(d < t);
    }

    #[test]
    fn numbers_format_without_noise() {
        assert_eq!(format_number(5.0), "5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-1.25), "-1.25");
    }

    #[test]
    fn emptiness_and_truthiness() {
        assert!(Value::Empty.is_empty());
        assert!(Value::List(vec![Value::Empty]).is_empty());
        assert!(!Value::Number(0.0).is_empty());
        assert!(!Value::Number(0.0).truthy());
        assert!(Value::text("x").truthy());
    }

    #[test]
    fn sort_puts_empty_first_and_ignores_case() {
        assert_eq!(Value::Empty.sort_cmp(&Value::Number(1.0)), Ordering::Less);
        assert_eq!(Value::text("a").sort_cmp(&Value::text("B")), Ordering::Less);
    }

    #[test]
    fn values_serialise_tagged() {
        let json = serde_json::to_string(&Value::Number(3.0)).unwrap_or_default();
        assert_eq!(json, r#"{"type":"number","value":3.0}"#);
    }
}
