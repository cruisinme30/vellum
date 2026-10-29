// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RFC 5545 recurrence rules for repeating tasks (R-MW-4).
//!
//! Tasks repeat by day, so this engine expands rules over calendar dates;
//! the time of day comes from the task's deadline. Supported parts:
//! `FREQ` (`DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`), `INTERVAL`, `COUNT`,
//! `UNTIL`, `BYDAY` (with ordinals for monthly and yearly rules),
//! `BYMONTHDAY` (negative counts from the month's end), `BYMONTH`,
//! `BYSETPOS` and `WKST`. Sub-daily frequencies and `BYYEARDAY`,
//! `BYWEEKNO`, `BYHOUR`, `BYMINUTE` and `BYSECOND` are refused, because a
//! task cannot repeat within a day.
//!
//! Completing a repeating task moves it forward rather than copying it
//! (D82): [`next_occurrence`] gives the new dates, counted from the
//! schedule (`every`) or from the completion day (`every!`).

use std::fmt;
use std::str::FromStr;

use jiff::ToSpan as _;
use jiff::civil::{Date, Weekday};

use crate::model::RecurFrom;

/// How often a rule repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Freq {
    /// Every day.
    Daily,
    /// Every week.
    Weekly,
    /// Every month.
    Monthly,
    /// Every year.
    Yearly,
}

impl Freq {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }
}

/// A `BYDAY` entry: a weekday, optionally the nth in the month or year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ByDay {
    /// `Some(2)` for the second, `Some(-1)` for the last; `None` for every.
    pub nth: Option<i8>,
    /// The weekday.
    pub weekday: Weekday,
}

/// A parsed recurrence rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RRule {
    /// The frequency.
    pub freq: Freq,
    /// Every `interval` periods; at least 1.
    pub interval: u32,
    /// How many occurrences remain, counting the current one.
    pub count: Option<u32>,
    /// The last day an occurrence may fall on.
    pub until: Option<Date>,
    /// `BYDAY`.
    pub by_day: Vec<ByDay>,
    /// `BYMONTHDAY`, 1 to 31 or -31 to -1.
    pub by_month_day: Vec<i8>,
    /// `BYMONTH`, 1 to 12.
    pub by_month: Vec<i8>,
    /// `BYSETPOS`, 1 to 366 or -366 to -1.
    pub by_set_pos: Vec<i16>,
    /// The first day of the week; Monday unless set.
    pub week_start: Weekday,
}

/// Why a rule could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RRuleError {
    /// A part is not `NAME=value`.
    #[error("malformed rule part {0:?}")]
    Malformed(String),
    /// `FREQ` is missing.
    #[error("the rule has no FREQ")]
    MissingFreq,
    /// A part or value Vellum tasks do not support.
    #[error("unsupported rule part {0:?}")]
    Unsupported(String),
    /// A value is out of range or not a number.
    #[error("invalid value in {0:?}")]
    Value(String),
    /// Both `COUNT` and `UNTIL` are set (RFC 5545 forbids it).
    #[error("COUNT and UNTIL cannot both be set")]
    CountAndUntil,
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("MO", Weekday::Monday),
    ("TU", Weekday::Tuesday),
    ("WE", Weekday::Wednesday),
    ("TH", Weekday::Thursday),
    ("FR", Weekday::Friday),
    ("SA", Weekday::Saturday),
    ("SU", Weekday::Sunday),
];

fn weekday_code(weekday: Weekday) -> &'static str {
    WEEKDAYS
        .iter()
        .find(|(_, w)| *w == weekday)
        .map_or("MO", |(code, _)| code)
}

fn parse_weekday(code: &str) -> Option<Weekday> {
    WEEKDAYS
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, w)| *w)
}

fn parse_list<T>(
    part: &str,
    value: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Vec<T>, RRuleError> {
    value
        .split(',')
        .map(|item| parse(item.trim()).ok_or_else(|| RRuleError::Value(part.to_owned())))
        .collect()
}

fn parse_signed_in<T: TryFrom<i32>>(text: &str, max: i32) -> Option<T> {
    let value: i32 = text.strip_prefix('+').unwrap_or(text).parse().ok()?;
    if value == 0 || value.abs() > max {
        return None;
    }
    T::try_from(value).ok()
}

fn parse_until(value: &str) -> Option<Date> {
    // DATE (20261103) or DATE-TIME (20261103T150000[Z]); tasks repeat by
    // day, so only the date matters.
    let digits = value.get(..8)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i16 = digits.get(..4)?.parse().ok()?;
    let month: i8 = digits.get(4..6)?.parse().ok()?;
    let day: i8 = digits.get(6..8)?.parse().ok()?;
    let rest = value.get(8..)?;
    if !(rest.is_empty() || rest.starts_with('T')) {
        return None;
    }
    Date::new(year, month, day).ok()
}

impl FromStr for RRule {
    type Err = RRuleError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let text = text
            .strip_prefix("RRULE:")
            .or_else(|| text.strip_prefix("rrule:"))
            .unwrap_or(text);
        let mut freq = None;
        let mut rule = Self {
            freq: Freq::Daily,
            interval: 1,
            count: None,
            until: None,
            by_day: Vec::new(),
            by_month_day: Vec::new(),
            by_month: Vec::new(),
            by_set_pos: Vec::new(),
            week_start: Weekday::Monday,
        };
        for part in text.split(';').filter(|p| !p.is_empty()) {
            let (name, value) = part
                .split_once('=')
                .ok_or_else(|| RRuleError::Malformed(part.to_owned()))?;
            let name = name.to_ascii_uppercase();
            let value = value.to_ascii_uppercase();
            match name.as_str() {
                "FREQ" => {
                    freq = Some(match value.as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return Err(RRuleError::Unsupported(part.to_owned())),
                    });
                }
                "INTERVAL" => {
                    rule.interval = value
                        .parse()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| RRuleError::Value(part.to_owned()))?;
                }
                "COUNT" => {
                    rule.count = Some(
                        value
                            .parse()
                            .ok()
                            .filter(|n| *n >= 1)
                            .ok_or_else(|| RRuleError::Value(part.to_owned()))?,
                    );
                }
                "UNTIL" => {
                    rule.until = Some(
                        parse_until(&value).ok_or_else(|| RRuleError::Value(part.to_owned()))?,
                    );
                }
                "BYDAY" => {
                    rule.by_day = parse_list(part, &value, |item| {
                        let split = item.len().checked_sub(2)?;
                        let weekday = parse_weekday(item.get(split..)?)?;
                        let prefix = item.get(..split)?;
                        let nth = if prefix.is_empty() {
                            None
                        } else {
                            Some(parse_signed_in::<i8>(prefix, 53)?)
                        };
                        Some(ByDay { nth, weekday })
                    })?;
                }
                "BYMONTHDAY" => {
                    rule.by_month_day = parse_list(part, &value, |v| parse_signed_in(v, 31))?;
                }
                "BYMONTH" => {
                    rule.by_month = parse_list(part, &value, |v| {
                        parse_signed_in::<i8>(v, 12).filter(|m| *m > 0)
                    })?;
                }
                "BYSETPOS" => {
                    rule.by_set_pos = parse_list(part, &value, |v| parse_signed_in(v, 366))?;
                }
                "WKST" => {
                    rule.week_start =
                        parse_weekday(&value).ok_or_else(|| RRuleError::Value(part.to_owned()))?;
                }
                _ => return Err(RRuleError::Unsupported(part.to_owned())),
            }
        }
        rule.freq = freq.ok_or(RRuleError::MissingFreq)?;
        if rule.count.is_some() && rule.until.is_some() {
            return Err(RRuleError::CountAndUntil);
        }
        let ordinal_allowed = matches!(rule.freq, Freq::Monthly | Freq::Yearly);
        if !ordinal_allowed && rule.by_day.iter().any(|d| d.nth.is_some()) {
            return Err(RRuleError::Value("BYDAY".to_owned()));
        }
        if rule.freq == Freq::Weekly && !rule.by_month_day.is_empty() {
            return Err(RRuleError::Value("BYMONTHDAY".to_owned()));
        }
        Ok(rule)
    }
}

fn join<T>(items: &[T], show: impl Fn(&T) -> String) -> String {
    items.iter().map(show).collect::<Vec<_>>().join(",")
}

impl fmt::Display for RRule {
    /// Writes the rule in a canonical part order, without `RRULE:`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FREQ={}", self.freq.as_str())?;
        if self.interval != 1 {
            write!(f, ";INTERVAL={}", self.interval)?;
        }
        if let Some(count) = self.count {
            write!(f, ";COUNT={count}")?;
        }
        if let Some(until) = self.until {
            write!(f, ";UNTIL={}", until.strftime("%Y%m%d"))?;
        }
        if !self.by_month.is_empty() {
            write!(f, ";BYMONTH={}", join(&self.by_month, ToString::to_string))?;
        }
        if !self.by_month_day.is_empty() {
            write!(
                f,
                ";BYMONTHDAY={}",
                join(&self.by_month_day, ToString::to_string)
            )?;
        }
        if !self.by_day.is_empty() {
            let days = join(&self.by_day, |d| {
                d.nth.map_or_else(String::new, |n| n.to_string()) + weekday_code(d.weekday)
            });
            write!(f, ";BYDAY={days}")?;
        }
        if !self.by_set_pos.is_empty() {
            write!(
                f,
                ";BYSETPOS={}",
                join(&self.by_set_pos, ToString::to_string)
            )?;
        }
        if self.week_start != Weekday::Monday {
            write!(f, ";WKST={}", weekday_code(self.week_start))?;
        }
        Ok(())
    }
}

/// Periods to scan without finding an occurrence before giving up, so a
/// rule that can never match (`BYMONTH=2;BYMONTHDAY=30`) ends.
const MAX_EMPTY_PERIODS: u32 = 2_000;

fn days_in_month(year: i16, month: i8) -> i8 {
    Date::new(year, month, 1).map_or(28, Date::days_in_month)
}

fn month_day(year: i16, month: i8, day: i8) -> Option<Date> {
    let len = days_in_month(year, month);
    let day = if day < 0 { len + 1 + day } else { day };
    if day < 1 || day > len {
        return None;
    }
    Date::new(year, month, day).ok()
}

/// Every date in `[first, last]` falling on `weekday`.
fn weekdays_between(first: Date, last: Date, weekday: Weekday) -> Vec<Date> {
    let mut out = Vec::new();
    let offset =
        (weekday.to_monday_zero_offset() - first.weekday().to_monday_zero_offset()).rem_euclid(7);
    let mut day = first.checked_add(i64::from(offset).days()).ok();
    while let Some(d) = day {
        if d > last {
            break;
        }
        out.push(d);
        day = d.checked_add(7.days()).ok();
    }
    out
}

/// Expands `BYDAY` within `[first, last]` (a month or a year), honouring
/// ordinals.
fn expand_by_day(first: Date, last: Date, by_day: &[ByDay]) -> Vec<Date> {
    let mut out = Vec::new();
    for entry in by_day {
        let all = weekdays_between(first, last, entry.weekday);
        match entry.nth {
            None => out.extend(all),
            Some(n) if n > 0 => {
                out.extend(
                    usize::try_from(n - 1)
                        .ok()
                        .and_then(|i| all.get(i))
                        .copied(),
                );
            }
            Some(n) => {
                let back = usize::try_from(-i32::from(n)).ok();
                out.extend(
                    back.and_then(|b| all.len().checked_sub(b))
                        .and_then(|i| all.get(i))
                        .copied(),
                );
            }
        }
    }
    out
}

impl RRule {
    fn month_allowed(&self, date: Date) -> bool {
        self.by_month.is_empty() || self.by_month.contains(&date.month())
    }

    fn month_day_allowed(&self, date: Date) -> bool {
        self.by_month_day.is_empty()
            || self
                .by_month_day
                .iter()
                .any(|d| month_day(date.year(), date.month(), *d).is_some_and(|m| m == date))
    }

    fn weekday_allowed(&self, date: Date) -> bool {
        self.by_day.is_empty() || self.by_day.iter().any(|d| d.weekday == date.weekday())
    }

    /// Candidate dates of one month for monthly and yearly rules.
    fn expand_month(&self, year: i16, month: i8, anchor: Date) -> Vec<Date> {
        let Ok(first) = Date::new(year, month, 1) else {
            return Vec::new();
        };
        let last = first.last_of_month();
        if self.by_day.is_empty() {
            if self.by_month_day.is_empty() {
                return month_day(year, month, anchor.day()).into_iter().collect();
            }
            return self
                .by_month_day
                .iter()
                .filter_map(|d| month_day(year, month, *d))
                .collect();
        }
        expand_by_day(first, last, &self.by_day)
            .into_iter()
            .filter(|d| self.month_day_allowed(*d))
            .collect()
    }

    /// Every candidate date in the period that starts at `period`.
    fn expand_period(&self, period: Date, anchor: Date) -> Vec<Date> {
        let mut set: Vec<Date> = match self.freq {
            Freq::Daily => {
                if self.month_allowed(period)
                    && self.month_day_allowed(period)
                    && self.weekday_allowed(period)
                {
                    vec![period]
                } else {
                    Vec::new()
                }
            }
            Freq::Weekly => {
                let days: Vec<Weekday> = if self.by_day.is_empty() {
                    vec![anchor.weekday()]
                } else {
                    self.by_day.iter().map(|d| d.weekday).collect()
                };
                (0..7)
                    .filter_map(|i| period.checked_add(i.days()).ok())
                    .filter(|d| days.contains(&d.weekday()) && self.month_allowed(*d))
                    .collect()
            }
            Freq::Monthly => {
                if self.month_allowed(period) {
                    self.expand_month(period.year(), period.month(), anchor)
                } else {
                    Vec::new()
                }
            }
            Freq::Yearly => {
                let year = period.year();
                let year_wide_by_day = self.by_month.is_empty()
                    && self.by_month_day.is_empty()
                    && !self.by_day.is_empty();
                if year_wide_by_day {
                    let first = period.first_of_year();
                    expand_by_day(first, first.last_of_year(), &self.by_day)
                } else {
                    let months: Vec<i8> = if self.by_month.is_empty() {
                        vec![anchor.month()]
                    } else {
                        self.by_month.clone()
                    };
                    let only_month_days = self.by_day.is_empty() && self.by_month_day.is_empty();
                    months
                        .into_iter()
                        .flat_map(|m| {
                            if only_month_days {
                                month_day(year, m, anchor.day()).into_iter().collect()
                            } else {
                                self.expand_month(year, m, anchor)
                            }
                        })
                        .collect()
                }
            }
        };
        set.sort_unstable();
        set.dedup();
        if self.by_set_pos.is_empty() {
            return set;
        }
        let len = set.len();
        let mut picked: Vec<Date> = self
            .by_set_pos
            .iter()
            .filter_map(|pos| {
                let index = if *pos > 0 {
                    usize::try_from(*pos - 1).ok()
                } else {
                    usize::try_from(-*pos).ok().and_then(|b| len.checked_sub(b))
                };
                index.and_then(|i| set.get(i)).copied()
            })
            .collect();
        picked.sort_unstable();
        picked.dedup();
        picked
    }

    /// The first day of the period containing `date`.
    fn period_start(&self, date: Date) -> Date {
        match self.freq {
            Freq::Daily => date,
            Freq::Weekly => {
                let back = (date.weekday().to_monday_zero_offset()
                    - self.week_start.to_monday_zero_offset())
                .rem_euclid(7);
                date.checked_sub(i64::from(back).days()).unwrap_or(date)
            }
            Freq::Monthly => date.first_of_month(),
            Freq::Yearly => date.first_of_year(),
        }
    }

    fn step(&self, period: Date) -> Option<Date> {
        let n = i64::from(self.interval);
        let span = match self.freq {
            Freq::Daily => n.days(),
            Freq::Weekly => n.weeks(),
            Freq::Monthly => n.months(),
            Freq::Yearly => n.years(),
        };
        period.checked_add(span).ok()
    }

    /// The occurrences on or after `dtstart`, in order, bounded by
    /// `COUNT` and `UNTIL`.
    #[must_use]
    pub fn occurrences(&self, dtstart: Date) -> Occurrences<'_> {
        Occurrences {
            rule: self,
            anchor: dtstart,
            period: Some(self.period_start(dtstart)),
            pending: Vec::new(),
            emitted: 0,
            empty_periods: 0,
        }
    }
}

/// Iterator returned by [`RRule::occurrences`].
#[derive(Debug)]
pub struct Occurrences<'a> {
    rule: &'a RRule,
    anchor: Date,
    period: Option<Date>,
    pending: Vec<Date>,
    emitted: u32,
    empty_periods: u32,
}

impl Iterator for Occurrences<'_> {
    type Item = Date;

    fn next(&mut self) -> Option<Date> {
        if self.rule.count.is_some_and(|c| self.emitted >= c) {
            return None;
        }
        while self.pending.is_empty() {
            let period = self.period?;
            if self.rule.until.is_some_and(|u| period > u) {
                self.period = None;
                return None;
            }
            let anchor = self.anchor;
            self.pending = self
                .rule
                .expand_period(period, anchor)
                .into_iter()
                .filter(|d| *d >= anchor)
                .rev()
                .collect();
            self.period = self.rule.step(period);
            if self.pending.is_empty() {
                self.empty_periods += 1;
                if self.empty_periods > MAX_EMPTY_PERIODS {
                    self.period = None;
                    return None;
                }
            } else {
                self.empty_periods = 0;
            }
        }
        let next = self.pending.pop()?;
        if self.rule.until.is_some_and(|u| next > u) {
            self.period = None;
            self.pending.clear();
            return None;
        }
        self.emitted += 1;
        Some(next)
    }
}

/// The dates of the next occurrence of a repeating task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextOccurrence {
    /// The new anchor date (the deadline's day, or the start date when
    /// the task has no deadline).
    pub date: Date,
    /// The rule to store with it (`COUNT` reduced by the occurrences used).
    pub rule: RRule,
}

/// Computes where a repeating task moves when it is completed on
/// `completed_on`.
///
/// `anchor` is the current occurrence: the deadline's day, or the start
/// date when there is no deadline. Returns `None` when the series is over.
#[must_use]
pub fn next_occurrence(
    rule: &RRule,
    from: RecurFrom,
    anchor: Date,
    completed_on: Date,
) -> Option<NextOccurrence> {
    match from {
        RecurFrom::Schedule => {
            // Skip occurrences already in the past: completing an overdue
            // daily task moves it to tomorrow, not to yesterday + 1.
            let floor = anchor.max(completed_on);
            let mut used = 0_u32;
            for date in rule.occurrences(anchor) {
                used += 1;
                if date > floor {
                    let mut rule = rule.clone();
                    if let Some(count) = rule.count {
                        // The occurrence at `date` is the first remaining.
                        rule.count = Some(count.checked_sub(used - 1)?);
                    }
                    return Some(NextOccurrence { date, rule });
                }
            }
            None
        }
        RecurFrom::Completion => {
            let mut next_rule = rule.clone();
            if let Some(count) = rule.count {
                // Each completion uses one occurrence, wherever it fell.
                next_rule.count = Some(count.checked_sub(1).filter(|c| *c > 0)?);
            }
            let unbounded = RRule {
                count: None,
                ..rule.clone()
            };
            let date = unbounded
                .occurrences(completed_on)
                .find(|d| *d > completed_on)?;
            Some(NextOccurrence {
                date,
                rule: next_rule,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn rule(text: &str) -> RRule {
        text.parse().unwrap()
    }

    fn take(text: &str, start: Date, n: usize) -> Vec<String> {
        rule(text)
            .occurrences(start)
            .take(n)
            .map(|d| d.to_string())
            .collect()
    }

    #[test]
    fn parses_and_writes_canonically() {
        let r = rule("RRULE:freq=monthly;byday=-1fr;interval=2;wkst=su");
        assert_eq!(r.to_string(), "FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR;WKST=SU");
        assert_eq!(rule(&r.to_string()), r);
        assert_eq!(
            "FREQ=HOURLY".parse::<RRule>(),
            Err(RRuleError::Unsupported("FREQ=HOURLY".into()))
        );
        assert_eq!("INTERVAL=2".parse::<RRule>(), Err(RRuleError::MissingFreq));
        assert!(
            "FREQ=DAILY;COUNT=2;UNTIL=20261231"
                .parse::<RRule>()
                .is_err()
        );
        assert!("FREQ=WEEKLY;BYDAY=1MO".parse::<RRule>().is_err());
        assert!("FREQ=DAILY;BYWEEKNO=3".parse::<RRule>().is_err());
        assert!("FREQ=DAILY;INTERVAL=0".parse::<RRule>().is_err());
    }

    #[test]
    fn daily_weekday_rule_skips_weekends() {
        // 2026-10-30 is a Friday.
        assert_eq!(
            take("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR", date(2026, 10, 30), 3),
            ["2026-10-30", "2026-11-02", "2026-11-03"]
        );
    }

    #[test]
    fn weekly_with_interval_and_days() {
        assert_eq!(
            take("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH", date(2026, 11, 3), 4),
            ["2026-11-03", "2026-11-05", "2026-11-17", "2026-11-19"]
        );
        assert_eq!(
            take("FREQ=WEEKLY", date(2026, 11, 4), 2),
            ["2026-11-04", "2026-11-11"]
        );
    }

    #[test]
    fn monthly_rules() {
        assert_eq!(
            take("FREQ=MONTHLY", date(2026, 1, 31), 3),
            ["2026-01-31", "2026-03-31", "2026-05-31"]
        );
        assert_eq!(
            take("FREQ=MONTHLY;BYMONTHDAY=-1", date(2026, 1, 31), 3),
            ["2026-01-31", "2026-02-28", "2026-03-31"]
        );
        assert_eq!(
            take("FREQ=MONTHLY;BYDAY=-1FR", date(2026, 11, 1), 2),
            ["2026-11-27", "2026-12-25"]
        );
        assert_eq!(
            take(
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
                date(2026, 10, 1),
                2
            ),
            ["2026-10-30", "2026-11-30"]
        );
        assert_eq!(
            take("FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13", date(2026, 1, 1), 2),
            ["2026-02-13", "2026-03-13"]
        );
    }

    #[test]
    fn yearly_rules() {
        assert_eq!(
            take("FREQ=YEARLY", date(2028, 2, 29), 2),
            ["2028-02-29", "2032-02-29"]
        );
        assert_eq!(
            take("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH", date(2026, 1, 1), 2),
            ["2026-11-26", "2027-11-25"]
        );
        assert_eq!(
            take("FREQ=YEARLY;BYDAY=1MO", date(2026, 1, 1), 2),
            ["2026-01-05", "2027-01-04"]
        );
    }

    #[test]
    fn count_and_until_bound_the_series() {
        assert_eq!(take("FREQ=DAILY;COUNT=2", date(2026, 1, 1), 5).len(), 2);
        assert_eq!(
            take("FREQ=WEEKLY;UNTIL=20260115T000000Z", date(2026, 1, 1), 5),
            ["2026-01-01", "2026-01-08", "2026-01-15"]
        );
        assert!(take("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30", date(2026, 1, 1), 1).is_empty());
    }

    #[test]
    fn schedule_recurrence_keeps_the_rhythm_and_skips_the_past() {
        let r = rule("FREQ=WEEKLY;BYDAY=MO");
        // Due Monday 2026-11-02, completed on time.
        let next = next_occurrence(
            &r,
            RecurFrom::Schedule,
            date(2026, 11, 2),
            date(2026, 11, 2),
        );
        assert_eq!(next.unwrap().date, date(2026, 11, 9));
        // Completed three weeks late: the next Monday after completion.
        let next = next_occurrence(
            &r,
            RecurFrom::Schedule,
            date(2026, 11, 2),
            date(2026, 11, 25),
        );
        assert_eq!(next.unwrap().date, date(2026, 11, 30));
        // Completed early: still the next scheduled Monday.
        let next = next_occurrence(
            &r,
            RecurFrom::Schedule,
            date(2026, 11, 2),
            date(2026, 10, 30),
        );
        assert_eq!(next.unwrap().date, date(2026, 11, 9));
    }

    #[test]
    fn completion_recurrence_counts_from_the_completion_day() {
        let r = rule("FREQ=WEEKLY;INTERVAL=2");
        let next = next_occurrence(
            &r,
            RecurFrom::Completion,
            date(2026, 11, 2),
            date(2026, 11, 5),
        );
        assert_eq!(next.unwrap().date, date(2026, 11, 19));
    }

    #[test]
    fn count_runs_out() {
        let r = rule("FREQ=DAILY;COUNT=2");
        let next =
            next_occurrence(&r, RecurFrom::Schedule, date(2026, 1, 1), date(2026, 1, 1)).unwrap();
        assert_eq!(next.date, date(2026, 1, 2));
        assert_eq!(next.rule.count, Some(1));
        assert_eq!(
            next_occurrence(&next.rule, RecurFrom::Schedule, next.date, next.date),
            None
        );
        let r = rule("FREQ=DAILY;COUNT=1");
        assert_eq!(
            next_occurrence(
                &r,
                RecurFrom::Completion,
                date(2026, 1, 1),
                date(2026, 1, 1)
            ),
            None
        );
    }
}
