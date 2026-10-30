// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Recurring issues (R-WK-12, D224).
//!
//! An issue with a due date can repeat every N days, weeks or months. It is
//! written in front matter as `repeat: 2w`. Once the due date has passed,
//! the daily run (at 00:01 team time) files the next issue: a copy of the
//! fields with the next due date. The repeat moves to the new issue, so the
//! old one never fires twice and the chain is one issue at a time.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{Date, Duration, Month};

use crate::error::WorkError;

/// The longest gap between two repeats, in units.
pub const MAX_EVERY: u16 = 366;

/// The unit a repeat counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unit {
    /// Days.
    Day,
    /// Weeks.
    Week,
    /// Calendar months; the day is clamped to the month's length.
    Month,
}

/// How often an issue repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Repeat {
    /// How many units between two issues, 1 to [`MAX_EVERY`].
    pub every: u16,
    /// The unit.
    pub unit: Unit,
}

impl Repeat {
    /// The due date after `from`.
    #[must_use]
    pub fn next(self, from: Date) -> Date {
        let n = i64::from(self.every);
        match self.unit {
            Unit::Day => from.saturating_add(Duration::days(n)),
            Unit::Week => from.saturating_add(Duration::weeks(n)),
            Unit::Month => add_months(from, u32::from(self.every)),
        }
    }

    /// The first due date after `due` that is not before `today`: a chain
    /// that missed runs catches up with one issue, not a backlog of them.
    #[must_use]
    pub fn next_on_or_after(self, due: Date, today: Date) -> Date {
        let mut next = self.next(due);
        // Bounded: one step is at least a day.
        for _ in 0..4000 {
            if next >= today {
                break;
            }
            next = self.next(next);
        }
        next
    }
}

fn add_months(from: Date, months: u32) -> Date {
    let index = u32::from(u8::from(from.month())) - 1 + months;
    let year = from
        .year()
        .saturating_add(i32::try_from(index / 12).unwrap_or(0));
    let month =
        Month::try_from(u8::try_from(index % 12 + 1).unwrap_or(1)).unwrap_or(Month::January);
    let day = from.day().min(month.length(year));
    Date::from_calendar_date(year, month, day).unwrap_or(from)
}

impl fmt::Display for Repeat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = match self.unit {
            Unit::Day => 'd',
            Unit::Week => 'w',
            Unit::Month => 'm',
        };
        write!(f, "{}{unit}", self.every)
    }
}

impl FromStr for Repeat {
    type Err = WorkError;

    /// Parses `3d`, `1w`, `2w` or `1m`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = |reason| WorkError::Invalid {
            what: "repeat",
            value: s.to_owned(),
            reason,
        };
        let text = s.trim();
        let (digits, unit) = text.split_at(text.len().saturating_sub(1));
        let unit = match unit {
            "d" | "D" => Unit::Day,
            "w" | "W" => Unit::Week,
            "m" | "M" => Unit::Month,
            _ => return Err(bad("write a number then d, w or m, such as 2w")),
        };
        let every: u16 = digits
            .parse()
            .map_err(|_| bad("write a number then d, w or m, such as 2w"))?;
        if every == 0 || every > MAX_EVERY {
            return Err(bad("the number must be between 1 and 366"));
        }
        Ok(Self { every, unit })
    }
}

impl Serialize for Repeat {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Repeat {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn r(s: &str) -> Repeat {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_prints() {
        for s in ["1d", "2w", "1m", "366d"] {
            assert_eq!(r(s).to_string(), s);
        }
        for bad in ["", "w", "0w", "2y", "-1d", "367d", "1.5w", "99999999w"] {
            assert!(bad.parse::<Repeat>().is_err(), "{bad}");
        }
    }

    #[test]
    fn steps_by_unit() {
        assert_eq!(r("3d").next(date!(2026 - 10 - 30)), date!(2026 - 11 - 02));
        assert_eq!(r("2w").next(date!(2026 - 10 - 30)), date!(2026 - 11 - 13));
        assert_eq!(r("1m").next(date!(2026 - 01 - 31)), date!(2026 - 02 - 28));
        assert_eq!(r("1m").next(date!(2028 - 01 - 31)), date!(2028 - 02 - 29));
        assert_eq!(r("12m").next(date!(2026 - 12 - 15)), date!(2027 - 12 - 15));
        assert_eq!(r("13m").next(date!(2026 - 12 - 15)), date!(2028 - 01 - 15));
    }

    #[test]
    fn catches_up_with_one_step() {
        let next = r("1w").next_on_or_after(date!(2026 - 09 - 01), date!(2026 - 10 - 29));
        assert_eq!(next, date!(2026 - 11 - 03));
        let next = r("1d").next_on_or_after(date!(2026 - 10 - 28), date!(2026 - 10 - 29));
        assert_eq!(next, date!(2026 - 10 - 29));
    }

    #[test]
    fn round_trips_in_yaml() {
        #[derive(Serialize, Deserialize)]
        struct F {
            repeat: Repeat,
        }
        let text = serde_saphyr::to_string(&F { repeat: r("2w") }).unwrap();
        assert!(text.contains("repeat: 2w"), "{text}");
        let back: F = serde_saphyr::from_str(&text).unwrap();
        assert_eq!(back.repeat, r("2w"));
    }
}
