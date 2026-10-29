// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Quick-add: one line of English into a task (R-MW-3).
//!
//! `Q` anywhere and `T` in a document open the quick-add box; as the
//! person types, the recognised phrases are highlighted from
//! [`QuickAdd::tokens`] and the rest becomes the title.
//!
//! | Phrase | Effect |
//! |---|---|
//! | `today`, `tomorrow`, `tmr`, `monday`, `next tue`, `on fri`, `nov 3`, `3rd november 2027`, `2026-11-03`, `in 2 weeks`, `next week`, `next month`, `this weekend`, `end of month` | deadline day |
//! | `3pm`, `3:30 pm`, `15:00`, `at 9`, `noon`, `midnight`, `in 2 hours` | deadline time |
//! | `tonight`, `this evening` | today, This evening |
//! | `start fri`, `starting nov 3`, `from tomorrow` | start date |
//! | `by fri`, `due nov 3` | deadline day (explicit) |
//! | `every day`, `daily`, `every weekday`, `every weekend`, `every 2 weeks`, `every mon, wed`, `every 3rd`, `every last fri`, `every nov 3`, `weekly`, `monthly`, `yearly` | repeat on schedule |
//! | `every! 2 weeks` | repeat from completion |
//! | `someday`, `anytime` | placement |
//! | `#list`, `@label`, `+person`, `p1`..`p4` | list, label, assignee, priority |
//! | `remind 15m before`, `remind me 1 hour before`, `remind at 9am` | reminder |
//!
//! Bare weekday abbreviations (`mon`, `wed`, `sat`, `sun`...) only count
//! after a prefix (`on`, `next`, `by`, `every`...), because they are also
//! words; full weekday names count anywhere. Weekdays always mean the next
//! one after today. Slash dates (`11/3`) are refused: their order depends
//! on the locale, and English packs come first (locale packs follow,
//! D84). Text in double quotes is never parsed. Only the first deadline,
//! start, time and repeat phrase count; later ones stay in the title.

use std::ops::Range;

use jiff::ToSpan as _;
use jiff::civil::{Date, DateTime, Time, Weekday};

use crate::model::{Due, Priority, RecurFrom, Recurrence, Reminder, TaskFields, When};
use crate::projection::next_weekday;
use crate::recur::{ByDay, Freq, RRule};
use crate::telemetry;

/// What a highlighted phrase set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// The deadline (day, time or both).
    Due,
    /// The start date.
    Start,
    /// A repeat rule.
    Recurrence,
    /// Someday, Anytime or This evening.
    When,
    /// `#list`.
    List,
    /// `@label`.
    Label,
    /// `+person`.
    Person,
    /// `p1` to `p4`.
    Priority,
    /// A reminder.
    Reminder,
}

/// One recognised phrase: its byte range in the input and what it set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Byte range in the input.
    pub span: Range<usize>,
    /// What it set.
    pub kind: TokenKind,
}

/// The result of parsing one quick-add line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickAdd {
    /// The title: the input without the recognised phrases.
    pub title: String,
    /// The fields the phrases set.
    pub fields: TaskFields,
    /// The list named with `#list`.
    pub list: Option<String>,
    /// The people named with `+person`.
    pub assignees: Vec<String>,
    /// The recognised phrases, in input order, for highlighting.
    pub tokens: Vec<Token>,
}

#[derive(Debug, Clone)]
struct Word {
    start: usize,
    end: usize,
    /// Lowercase, without trailing `,;.?!` (except in `every!`).
    key: String,
    literal: bool,
}

fn words(input: &str) -> Vec<Word> {
    let mut quoted: Vec<Range<usize>> = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in input.char_indices() {
        if c == '"' {
            match open.take() {
                Some(start) => quoted.push(start..i + 1),
                None => open = Some(i),
            }
        }
    }
    if let Some(start) = open {
        quoted.push(start..input.len());
    }
    let mut out = Vec::new();
    let mut start = None;
    let push = |out: &mut Vec<Word>, s: usize, e: usize| {
        let raw = input.get(s..e).unwrap_or_default().to_lowercase();
        let key = if raw == "every!" {
            raw
        } else {
            raw.trim_end_matches([',', ';', '.', '?', '!']).to_owned()
        };
        let literal = quoted.iter().any(|q| q.contains(&s));
        out.push(Word {
            start: s,
            end: e,
            key,
            literal,
        });
    };
    for (i, c) in input.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                push(&mut out, s, i);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        push(&mut out, s, input.len());
    }
    out
}

const WEEKDAY_NAMES: [(&str, Weekday, bool); 17] = [
    ("monday", Weekday::Monday, true),
    ("tuesday", Weekday::Tuesday, true),
    ("wednesday", Weekday::Wednesday, true),
    ("thursday", Weekday::Thursday, true),
    ("friday", Weekday::Friday, true),
    ("saturday", Weekday::Saturday, true),
    ("sunday", Weekday::Sunday, true),
    ("mon", Weekday::Monday, false),
    ("tue", Weekday::Tuesday, false),
    ("tues", Weekday::Tuesday, false),
    ("wed", Weekday::Wednesday, false),
    ("weds", Weekday::Wednesday, false),
    ("thu", Weekday::Thursday, false),
    ("thur", Weekday::Thursday, false),
    ("thurs", Weekday::Thursday, false),
    ("fri", Weekday::Friday, false),
    ("sat", Weekday::Saturday, false),
];

fn weekday(key: &str, prefixed: bool) -> Option<Weekday> {
    if key == "sun" {
        return prefixed.then_some(Weekday::Sunday);
    }
    WEEKDAY_NAMES
        .iter()
        .find(|(name, _, _)| *name == key)
        .filter(|(_, _, standalone)| *standalone || prefixed)
        .map(|(_, day, _)| *day)
}

fn month(key: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    if key == "sept" {
        return Some(9);
    }
    if key.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| m.starts_with(key))
        .and_then(|i| i8::try_from(i + 1).ok())
}

fn day_number(key: &str) -> Option<i8> {
    let digits = key
        .strip_suffix("st")
        .or_else(|| key.strip_suffix("nd"))
        .or_else(|| key.strip_suffix("rd"))
        .or_else(|| key.strip_suffix("th"))
        .unwrap_or(key);
    let n: i8 = digits.parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}

fn ordinal(key: &str) -> Option<i8> {
    match key {
        "first" => Some(1),
        "second" => Some(2),
        "third" => Some(3),
        "fourth" => Some(4),
        "last" => Some(-1),
        _ => {
            let suffixed = ["st", "nd", "rd", "th"].iter().any(|s| key.ends_with(s));
            if suffixed { day_number(key) } else { None }
        }
    }
}

fn number(key: &str) -> Option<u32> {
    match key {
        "a" | "an" | "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        _ => key.parse().ok().filter(|n| (1..=999).contains(n)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

fn unit(key: &str) -> Option<Unit> {
    match key {
        "m" | "min" | "mins" | "minute" | "minutes" => Some(Unit::Minute),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(Unit::Hour),
        "d" | "day" | "days" => Some(Unit::Day),
        "w" | "wk" | "wks" | "week" | "weeks" => Some(Unit::Week),
        "mo" | "month" | "months" => Some(Unit::Month),
        "y" | "yr" | "yrs" | "year" | "years" => Some(Unit::Year),
        _ => None,
    }
}

/// `15m`, `2h` written as one word.
fn compact_duration(key: &str) -> Option<(u32, Unit)> {
    let split = key.find(|c: char| !c.is_ascii_digit())?;
    let n = number(key.get(..split)?)?;
    Some((n, unit(key.get(split..)?)?))
}

const fn simple_rule(freq: Freq) -> RRule {
    RRule {
        freq,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        by_set_pos: Vec::new(),
        week_start: Weekday::Monday,
    }
}

const fn freq_of(unit: Unit) -> Option<Freq> {
    match unit {
        Unit::Day => Some(Freq::Daily),
        Unit::Week => Some(Freq::Weekly),
        Unit::Month => Some(Freq::Monthly),
        Unit::Year => Some(Freq::Yearly),
        Unit::Minute | Unit::Hour => None,
    }
}

/// What a date phrase found.
#[derive(Debug, Clone, Copy)]
enum DateHit {
    Day(Date, Option<When>),
    Moment(DateTime),
}

struct Parser<'a> {
    words: &'a [Word],
    now: DateTime,
}

impl Parser<'_> {
    fn key(&self, i: usize) -> Option<&str> {
        self.words
            .get(i)
            .filter(|w| !w.literal)
            .map(|w| w.key.as_str())
    }

    fn today(&self) -> Date {
        self.now.date()
    }

    fn add(&self, n: u32, unit: Unit) -> Option<DateHit> {
        let n = i64::from(n);
        match unit {
            Unit::Minute => self.now.checked_add(n.minutes()).ok().map(DateHit::Moment),
            Unit::Hour => self.now.checked_add(n.hours()).ok().map(DateHit::Moment),
            Unit::Day => self
                .today()
                .checked_add(n.days())
                .ok()
                .map(|d| DateHit::Day(d, None)),
            Unit::Week => self
                .today()
                .checked_add(n.weeks())
                .ok()
                .map(|d| DateHit::Day(d, None)),
            Unit::Month => self
                .today()
                .checked_add(n.months())
                .ok()
                .map(|d| DateHit::Day(d, None)),
            Unit::Year => self
                .today()
                .checked_add(n.years())
                .ok()
                .map(|d| DateHit::Day(d, None)),
        }
    }

    /// A month-and-day phrase at `i`: `nov 3`, `november 3rd 2027`,
    /// `3 nov`, `3rd of november`.
    fn month_day(&self, i: usize) -> Option<(usize, Date)> {
        let today = self.today();
        let (m, d, mut used) = if let Some(m) = self.key(i).and_then(month) {
            let d = self.key(i + 1).and_then(day_number)?;
            (m, d, 2)
        } else {
            let d = self.key(i).and_then(day_number)?;
            let of = usize::from(self.key(i + 1) == Some("of"));
            let m = self.key(i + 1 + of).and_then(month)?;
            (m, d, 2 + of)
        };
        let explicit_year = self
            .key(i + used)
            .and_then(|k| k.parse::<i16>().ok())
            .filter(|y| (1970..=2200).contains(y));
        let date = if let Some(year) = explicit_year {
            used += 1;
            Date::new(year, m, d).ok()?
        } else {
            let this_year = Date::new(today.year(), m, d).ok();
            match this_year {
                Some(date) if date >= today => date,
                _ => Date::new(today.year() + 1, m, d).ok()?,
            }
        };
        Some((used, date))
    }

    /// A date phrase at `i`; `prefixed` says a prefix word came before.
    fn date(&self, i: usize, prefixed: bool) -> Option<(usize, DateHit)> {
        let key = self.key(i)?;
        let today = self.today();
        let day = |d: Date| DateHit::Day(d, None);
        let hit = match key {
            "today" | "tod" => (1, day(today)),
            "tonight" => (1, DateHit::Day(today, Some(When::Evening))),
            "tomorrow" | "tmr" | "tmrw" | "tomorow" => (1, day(today.checked_add(1.day()).ok()?)),
            "this" => match self.key(i + 1)? {
                "evening" => (2, DateHit::Day(today, Some(When::Evening))),
                "weekend" => (2, day(self.weekend(false))),
                other => (2, day(next_weekday(today, weekday(other, true)?))),
            },
            "weekend" if prefixed => (1, day(self.weekend(false))),
            "next" => match self.key(i + 1)? {
                "week" => (2, day(next_weekday(today, Weekday::Monday))),
                "month" => (2, day(today.first_of_month().checked_add(1.month()).ok()?)),
                "year" => (2, day(Date::new(today.year() + 1, 1, 1).ok()?)),
                "weekend" => (2, day(self.weekend(true))),
                other => (2, day(next_weekday(today, weekday(other, true)?))),
            },
            "in" => {
                if let Some((n, unit)) = self.key(i + 1).and_then(compact_duration) {
                    (2, self.add(n, unit)?)
                } else {
                    let n = self.key(i + 1).and_then(number)?;
                    let unit = self.key(i + 2).and_then(unit)?;
                    (3, self.add(n, unit)?)
                }
            }
            "end" => {
                if self.key(i + 1) != Some("of") {
                    return None;
                }
                match self.key(i + 2)? {
                    "month" => (3, day(today.last_of_month())),
                    "week" => (3, day(self.on_or_after(Weekday::Friday))),
                    "year" => (3, day(today.last_of_year())),
                    _ => return None,
                }
            }
            "eom" => (1, day(today.last_of_month())),
            "eow" => (1, day(self.on_or_after(Weekday::Friday))),
            _ => {
                if let Some(wd) = weekday(key, prefixed) {
                    (1, day(next_weekday(today, wd)))
                } else if let Ok(date) = key.parse::<Date>() {
                    (1, day(date))
                } else {
                    let (used, date) = self.month_day(i)?;
                    (used, day(date))
                }
            }
        };
        Some(hit)
    }

    fn on_or_after(&self, wd: Weekday) -> Date {
        let today = self.today();
        if today.weekday() == wd {
            today
        } else {
            next_weekday(today, wd)
        }
    }

    fn weekend(&self, next: bool) -> Date {
        let saturday = self.on_or_after(Weekday::Saturday);
        let this = if self.today().weekday() == Weekday::Sunday {
            self.today()
        } else {
            saturday
        };
        if next {
            next_weekday(this.checked_add(1.day()).unwrap_or(this), Weekday::Saturday)
        } else {
            this
        }
    }

    /// A time phrase at `i`: `3pm`, `3:30 pm`, `15:00`, `at 9`, `noon`.
    fn time(&self, i: usize) -> Option<(usize, Time)> {
        let key = self.key(i)?;
        if key == "at" || key == "@" {
            let (used, time) = self.time(i + 1).or_else(|| {
                let hour: i8 = self.key(i + 1)?.parse().ok()?;
                Some((1, Time::new(hour, 0, 0, 0).ok()?))
            })?;
            return Some((used + 1, time));
        }
        match key {
            "noon" | "midday" => return Some((1, Time::constant(12, 0, 0, 0))),
            "midnight" => return Some((1, Time::constant(23, 59, 0, 0))),
            _ => {}
        }
        let (body, meridiem, used) = if let Some(body) = key.strip_suffix("am") {
            (body, Some(false), 1)
        } else if let Some(body) = key.strip_suffix("pm") {
            (body, Some(true), 1)
        } else {
            match self.key(i + 1) {
                Some("am" | "a.m") => (key, Some(false), 2),
                Some("pm" | "p.m") => (key, Some(true), 2),
                _ if key.contains(':') => (key, None, 1),
                _ => return None,
            }
        };
        let (hour, minute) = match body.split_once(':') {
            Some((h, m)) if m.len() == 2 => (h.parse::<i8>().ok()?, m.parse::<i8>().ok()?),
            Some(_) => return None,
            None => (body.parse::<i8>().ok()?, 0),
        };
        let hour = match meridiem {
            Some(pm) if (1..=12).contains(&hour) => hour % 12 + if pm { 12 } else { 0 },
            Some(_) => return None,
            None => hour,
        };
        Some((used, Time::new(hour, minute, 0, 0).ok()?))
    }

    /// The weekdays of `every mon, wed and fri`.
    fn weekday_list(&self, start: usize) -> (usize, Vec<Weekday>) {
        let mut i = start;
        let mut days = Vec::new();
        while let Some(wd) = self.key(i).and_then(|k| weekday(k, true)) {
            if !days.contains(&wd) {
                days.push(wd);
            }
            i += 1;
            let joined = matches!(self.key(i), Some("and" | "&"));
            if joined && self.key(i + 1).and_then(|k| weekday(k, true)).is_some() {
                i += 1;
            }
        }
        (i - start, days)
    }

    /// A repeat phrase at `i`.
    fn recurrence(&self, i: usize) -> Option<(usize, Recurrence)> {
        let schedule = |rule| Recurrence {
            rule,
            from: RecurFrom::Schedule,
        };
        let key = self.key(i)?;
        match key {
            "daily" => return Some((1, schedule(simple_rule(Freq::Daily)))),
            "weekly" => return Some((1, schedule(simple_rule(Freq::Weekly)))),
            "monthly" => return Some((1, schedule(simple_rule(Freq::Monthly)))),
            "yearly" | "annually" => return Some((1, schedule(simple_rule(Freq::Yearly)))),
            "every" | "every!" => {}
            _ => return None,
        }
        let from = if key == "every!" {
            RecurFrom::Completion
        } else {
            RecurFrom::Schedule
        };
        let (used, rule) = self
            .every_interval(i + 1)
            .or_else(|| self.every_days(i + 1))
            .or_else(|| self.every_ordinal(i + 1))?;
        Some((1 + used, Recurrence { rule, from }))
    }

    /// After `every`: `day`, `2 weeks`, `2w`, `other month`.
    fn every_interval(&self, i: usize) -> Option<(usize, RRule)> {
        let next = self.key(i)?;
        if let Some(freq) = unit(next).and_then(freq_of) {
            return Some((1, simple_rule(freq)));
        }
        let (used, interval, freq) = if let Some((n, u)) = compact_duration(next) {
            (1, n, freq_of(u)?)
        } else if next == "other" {
            (2, 2, self.key(i + 1).and_then(unit).and_then(freq_of)?)
        } else {
            let n = number(next)
                .filter(|_| next.bytes().all(|b| b.is_ascii_digit()) || next.len() > 2)?;
            (2, n, self.key(i + 1).and_then(unit).and_then(freq_of)?)
        };
        let mut rule = simple_rule(freq);
        rule.interval = interval;
        Some((used, rule))
    }

    /// After `every`: `weekday`, `weekend`, `mon, wed and fri`.
    fn every_days(&self, i: usize) -> Option<(usize, RRule)> {
        let every = |freq, days: &[Weekday]| {
            let mut rule = simple_rule(freq);
            rule.by_day = days
                .iter()
                .map(|weekday| ByDay {
                    nth: None,
                    weekday: *weekday,
                })
                .collect();
            rule
        };
        match self.key(i)? {
            "weekday" | "workday" => Some((
                1,
                every(
                    Freq::Daily,
                    &[
                        Weekday::Monday,
                        Weekday::Tuesday,
                        Weekday::Wednesday,
                        Weekday::Thursday,
                        Weekday::Friday,
                    ],
                ),
            )),
            "weekend" => Some((
                1,
                every(Freq::Weekly, &[Weekday::Saturday, Weekday::Sunday]),
            )),
            _ => {
                let (used, days) = self.weekday_list(i);
                (used > 0).then(|| (used, every(Freq::Weekly, &days)))
            }
        }
    }

    /// After `every`: `3rd`, `last fri`, `nov 3`.
    fn every_ordinal(&self, i: usize) -> Option<(usize, RRule)> {
        if let Some(nth) = self.key(i).and_then(ordinal) {
            let mut rule = simple_rule(Freq::Monthly);
            if let Some(weekday) = self.key(i + 1).and_then(|k| weekday(k, true)) {
                rule.by_day = vec![ByDay {
                    nth: Some(nth),
                    weekday,
                }];
                return Some((2, rule));
            }
            rule.by_month_day = vec![nth];
            return Some((1 + usize::from(self.key(i + 1) == Some("day")), rule));
        }
        let (used, date) = self.month_day(i)?;
        let mut rule = simple_rule(Freq::Yearly);
        rule.by_month = vec![date.month()];
        rule.by_month_day = vec![date.day()];
        Some((used, rule))
    }

    /// A reminder phrase at `i`: `remind [me] 15m before`, `remind [me] at 9am`.
    fn reminder(&self, i: usize, due: Option<Date>) -> Option<(usize, Reminder)> {
        if self.key(i)? != "remind" {
            return None;
        }
        let me = usize::from(self.key(i + 1) == Some("me"));
        let j = i + 1 + me;
        if self.key(j) == Some("at")
            && let Some((used, time)) = self.time(j)
        {
            let day = due.unwrap_or_else(|| self.today());
            return Some((1 + me + used, Reminder::At(day.to_datetime(time))));
        }
        let (n, u, used) = if let Some((n, u)) = self.key(j).and_then(compact_duration) {
            (n, u, 1)
        } else {
            (
                self.key(j).and_then(number)?,
                self.key(j + 1).and_then(unit)?,
                2,
            )
        };
        if self.key(j + used) != Some("before") {
            return None;
        }
        let n = i64::from(n);
        let span = match u {
            Unit::Minute => jiff::SignedDuration::from_mins(n),
            Unit::Hour => jiff::SignedDuration::from_hours(n),
            Unit::Day => jiff::SignedDuration::from_hours(n * 24),
            Unit::Week => jiff::SignedDuration::from_hours(n * 24 * 7),
            Unit::Month | Unit::Year => return None,
        };
        Some((1 + me + used + 1, Reminder::Relative(-span)))
    }
}

#[derive(Default)]
struct State {
    due_day: Option<Date>,
    due_time: Option<Time>,
    due_moment: Option<DateTime>,
    start: Option<Date>,
    when: Option<When>,
    recurrence: Option<Recurrence>,
}

/// The parse in progress.
struct Run<'a> {
    parser: Parser<'a>,
    input: &'a str,
    out: QuickAdd,
    state: State,
}

impl Run<'_> {
    const fn has_due(&self) -> bool {
        self.state.due_day.is_some() || self.state.due_moment.is_some()
    }

    const fn time_free(&self) -> bool {
        self.state.due_time.is_none() && self.state.due_moment.is_none()
    }

    /// Takes a time right after a date phrase, if there is one.
    fn trailing_time(&mut self, at: usize) -> usize {
        if !self.time_free() {
            return 0;
        }
        match self.parser.time(at) {
            Some((used, time)) => {
                self.state.due_time = Some(time);
                used
            }
            None => 0,
        }
    }

    /// Tries every phrase at word `i`; returns how many words it used
    /// and what it set.
    fn step(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        self.repeat(i)
            .or_else(|| self.prefixed_date(i))
            .or_else(|| self.bare_date(i))
            .or_else(|| self.bare_time(i))
            .or_else(|| self.reminder(i))
            .or_else(|| self.word(i))
    }

    /// Repeat phrases, optionally followed by a time.
    fn repeat(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        if self.state.recurrence.is_some() {
            return None;
        }
        let (used, recurrence) = self.parser.recurrence(i)?;
        self.state.recurrence = Some(recurrence);
        let used = used + self.trailing_time(i + used);
        Some((used, TokenKind::Recurrence))
    }

    /// Prefixed dates: `start`, `from`, `by`, `due`, `on`.
    fn prefixed_date(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        let key = self.parser.key(i)?;
        let kind = match key {
            "start" | "starting" | "starts" | "from" => TokenKind::Start,
            "by" | "due" | "on" | "before" => TokenKind::Due,
            _ => return None,
        };
        let skip = usize::from(key == "due" && self.parser.key(i + 1) == Some("on"));
        let free = match kind {
            TokenKind::Start => self.state.start.is_none(),
            _ => !self.has_due(),
        };
        if !free {
            return None;
        }
        let (used, hit) = self.parser.date(i + 1 + skip, true)?;
        let used = 1 + skip + used;
        let state = &mut self.state;
        match (kind, hit) {
            (TokenKind::Start, DateHit::Day(d, when)) => {
                state.start = Some(d);
                state.when = state.when.or(when);
            }
            (TokenKind::Start, DateHit::Moment(m)) => state.start = Some(m.date()),
            (_, DateHit::Day(d, when)) => {
                state.due_day = Some(d);
                state.when = state.when.or(when);
            }
            (_, DateHit::Moment(m)) => state.due_moment = Some(m),
        }
        let used = if kind == TokenKind::Due {
            used + self.trailing_time(i + used)
        } else {
            used
        };
        Some((used, kind))
    }

    /// Bare dates, optionally followed by a time.
    fn bare_date(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        if self.has_due() {
            return None;
        }
        let (used, hit) = self.parser.date(i, false)?;
        let state = &mut self.state;
        let kind = match hit {
            DateHit::Day(d, Some(When::Evening)) => {
                // "tonight" plans the task for this evening; it is not a
                // deadline.
                state.start = state.start.or(Some(d));
                state.when = Some(When::Evening);
                TokenKind::When
            }
            DateHit::Day(d, when) => {
                state.due_day = Some(d);
                state.when = state.when.or(when);
                TokenKind::Due
            }
            DateHit::Moment(m) => {
                state.due_moment = Some(m);
                TokenKind::Due
            }
        };
        Some((used + self.trailing_time(i + used), kind))
    }

    /// Bare times, optionally followed by a date.
    fn bare_time(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        if !self.time_free() {
            return None;
        }
        let (mut used, time) = self.parser.time(i)?;
        self.state.due_time = Some(time);
        if !self.has_due()
            && let Some((d_used, DateHit::Day(d, when))) = self.parser.date(i + used, false)
        {
            self.state.due_day = Some(d);
            self.state.when = self.state.when.or(when);
            used += d_used;
        }
        Some((used, TokenKind::Due))
    }

    fn reminder(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        if !self.out.fields.reminders.is_empty() {
            return None;
        }
        let (used, reminder) = self.parser.reminder(i, self.state.due_day)?;
        self.out.fields.reminders.push(reminder);
        Some((used, TokenKind::Reminder))
    }

    /// One-word phrases: placement, sigils and priority.
    fn word(&mut self, i: usize) -> Option<(usize, TokenKind)> {
        let word = self.parser.words.get(i)?;
        let key = word.key.as_str();
        match key {
            "someday" | "anytime" if self.state.when.is_none() => {
                self.state.when = Some(if key == "someday" {
                    When::Someday
                } else {
                    When::Anytime
                });
                return Some((1, TokenKind::When));
            }
            _ => {}
        }
        let raw = self.input.get(word.start..word.end).unwrap_or_default();
        let sigil = |prefix: char| -> Option<String> {
            let name = raw
                .strip_prefix(prefix)?
                .trim_end_matches([',', ';', '.', '?', '!']);
            (!name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
            .then(|| name.to_owned())
        };
        if self.out.list.is_none()
            && let Some(name) = sigil('#')
        {
            self.out.list = Some(name);
            return Some((1, TokenKind::List));
        }
        if let Some(name) = sigil('@') {
            self.out.fields.labels.push(name.to_lowercase());
            return Some((1, TokenKind::Label));
        }
        if let Some(name) = sigil('+') {
            self.out.assignees.push(name.to_lowercase());
            return Some((1, TokenKind::Person));
        }
        if self.out.fields.priority.is_none()
            && let Some(level) = key
                .strip_prefix('p')
                .and_then(|n| n.parse::<u8>().ok())
                .and_then(Priority::new)
        {
            self.out.fields.priority = Some(level);
            return Some((1, TokenKind::Priority));
        }
        None
    }
}

/// Parses one quick-add line. `now` is the owner's local date and time.
#[must_use]
pub fn parse(input: &str, now: DateTime) -> QuickAdd {
    let _span = tracing::debug_span!("tasks.quick_add", len = input.len()).entered();
    let words = words(input);
    let mut run = Run {
        parser: Parser { words: &words, now },
        input,
        out: QuickAdd::default(),
        state: State::default(),
    };
    let mut i = 0;
    while i < words.len() {
        if words.get(i).is_some_and(|w| w.literal) {
            i += 1;
            continue;
        }
        match run.step(i) {
            Some((used, kind)) => {
                let used = used.max(1);
                let start = words.get(i).map_or(0, |w| w.start);
                let end = words.get(i + used - 1).map_or(input.len(), |w| w.end);
                run.out.tokens.push(Token {
                    span: start..end,
                    kind,
                });
                i += used;
            }
            None => i += 1,
        }
    }
    let Run { mut out, state, .. } = run;
    finish(&mut out, state, now);
    out.title = title(input, &out.tokens);
    telemetry::record_quick_add(out.fields.due.is_some() || out.fields.start.is_some());
    out
}

fn finish(out: &mut QuickAdd, state: State, now: DateTime) {
    let today = now.date();
    let mut due_day = state.due_day;
    if let Some(recurrence) = &state.recurrence
        && due_day.is_none()
        && state.due_moment.is_none()
    {
        // The first occurrence on or after today.
        due_day = recurrence.rule.occurrences(today).next();
    }
    out.fields.due = match (state.due_moment, due_day, state.due_time) {
        (Some(moment), _, _) => Some(Due::DateTime(moment)),
        (None, Some(day), Some(time)) => Some(Due::DateTime(day.to_datetime(time))),
        (None, Some(day), None) => Some(Due::Date(day)),
        (None, None, Some(time)) => {
            // A time alone means the next time the clock shows it.
            let day = if time > now.time() {
                today
            } else {
                today.tomorrow().unwrap_or(today)
            };
            Some(Due::DateTime(day.to_datetime(time)))
        }
        (None, None, None) => None,
    };
    out.fields.start = state.start;
    out.fields.when = state.when;
    out.fields.recurrence = state.recurrence;
    if let (Some(Due::Date(day)), Some(Reminder::At(at))) =
        (out.fields.due, out.fields.reminders.first_mut())
        && at.date() == today
    {
        // "remind at 9am" before the deadline was parsed: move to its day.
        *at = day.to_datetime(at.time());
    }
}

/// The input with the token spans removed and spaces collapsed.
#[must_use]
pub fn title(input: &str, tokens: &[Token]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut at = 0;
    for token in tokens {
        out.push_str(input.get(at..token.span.start).unwrap_or_default());
        out.push(' ');
        at = token.span.end;
    }
    out.push_str(input.get(at..).unwrap_or_default());
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    /// Monday 2026-11-02, 10:30.
    fn now() -> DateTime {
        date(2026, 11, 2).at(10, 30, 0, 0)
    }

    fn due(text: &str) -> Option<String> {
        parse(text, now()).fields.due.map(|d| d.to_string())
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(
            due("Call Ana tomorrow 3pm").as_deref(),
            Some("2026-11-03T15:00")
        );
        assert_eq!(
            due("Call Ana at 9 tomorrow").as_deref(),
            Some("2026-11-03T09:00")
        );
        assert_eq!(due("Pay rent next tue").as_deref(), Some("2026-11-03"));
        assert_eq!(due("Pay rent on mon").as_deref(), Some("2026-11-09"));
        assert_eq!(due("Pay rent monday").as_deref(), Some("2026-11-09"));
        assert_eq!(due("Sat with the team").as_deref(), None);
        assert_eq!(due("Report nov 3").as_deref(), Some("2026-11-03"));
        assert_eq!(due("Report 1st November").as_deref(), Some("2027-11-01"));
        assert_eq!(
            due("Report 3rd of march 2027").as_deref(),
            Some("2027-03-03")
        );
        assert_eq!(due("Report 2026-12-24").as_deref(), Some("2026-12-24"));
        assert_eq!(due("Renew in 2 weeks").as_deref(), Some("2026-11-16"));
        assert_eq!(
            due("Check oven in 2 hours").as_deref(),
            Some("2026-11-02T12:30")
        );
        assert_eq!(due("Plan next week").as_deref(), Some("2026-11-09"));
        assert_eq!(due("Plan next month").as_deref(), Some("2026-12-01"));
        assert_eq!(
            due("Close books end of month").as_deref(),
            Some("2026-11-30")
        );
        assert_eq!(due("Stand-up 9:15am").as_deref(), Some("2026-11-03T09:15"));
        assert_eq!(due("Lunch noon").as_deref(), Some("2026-11-02T12:00"));
        assert_eq!(due("Read chapter 3").as_deref(), None);
        assert_eq!(due("May the best win").as_deref(), None);
    }

    #[test]
    fn the_prd_examples() {
        let q = parse("Ship deck every weekday 9am #work @focus p1 +ann", now());
        assert_eq!(q.title, "Ship deck");
        let rec = q.fields.recurrence.unwrap();
        assert_eq!(rec.rule.to_string(), "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR");
        assert_eq!(rec.from, RecurFrom::Schedule);
        assert_eq!(q.fields.due.unwrap().to_string(), "2026-11-02T09:00");
        assert_eq!(q.list.as_deref(), Some("work"));
        assert_eq!(q.fields.labels, ["focus"]);
        assert_eq!(q.fields.priority, Priority::new(1));
        assert_eq!(q.assignees, ["ann"]);
        let kinds: Vec<TokenKind> = q.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TokenKind::Recurrence,
                TokenKind::List,
                TokenKind::Label,
                TokenKind::Priority,
                TokenKind::Person
            ]
        );

        let q = parse("Water plants every! 2 weeks", now());
        let rec = q.fields.recurrence.unwrap();
        assert_eq!(rec.from, RecurFrom::Completion);
        assert_eq!(rec.rule.to_string(), "FREQ=WEEKLY;INTERVAL=2");
        assert_eq!(q.title, "Water plants");
    }

    #[test]
    fn repeat_phrases() {
        let rule = |text: &str| {
            parse(text, now())
                .fields
                .recurrence
                .map(|r| r.rule.to_string())
        };
        assert_eq!(rule("x every day").as_deref(), Some("FREQ=DAILY"));
        assert_eq!(rule("x daily").as_deref(), Some("FREQ=DAILY"));
        assert_eq!(
            rule("x every other week").as_deref(),
            Some("FREQ=WEEKLY;INTERVAL=2")
        );
        assert_eq!(
            rule("x every 3 months").as_deref(),
            Some("FREQ=MONTHLY;INTERVAL=3")
        );
        assert_eq!(
            rule("x every mon, wed and fri").as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,WE,FR")
        );
        assert_eq!(
            rule("x every 3rd").as_deref(),
            Some("FREQ=MONTHLY;BYMONTHDAY=3")
        );
        assert_eq!(
            rule("x every last fri").as_deref(),
            Some("FREQ=MONTHLY;BYDAY=-1FR")
        );
        assert_eq!(
            rule("x every nov 3").as_deref(),
            Some("FREQ=YEARLY;BYMONTH=11;BYMONTHDAY=3")
        );
        assert_eq!(
            rule("x every weekend").as_deref(),
            Some("FREQ=WEEKLY;BYDAY=SA,SU")
        );
        assert_eq!(rule("x every so often").as_deref(), None);
        let q = parse("Bins every thursday", now());
        assert_eq!(q.fields.due.unwrap().to_string(), "2026-11-05");
    }

    #[test]
    fn placement_start_and_reminders() {
        let q = parse("Read tonight", now());
        assert_eq!(q.fields.when, Some(When::Evening));
        assert_eq!(q.fields.start, Some(date(2026, 11, 2)));
        assert_eq!(q.fields.due, None);
        let q = parse("Learn Rust someday", now());
        assert_eq!(q.fields.when, Some(When::Someday));
        let q = parse(
            "Draft plan starting fri by nov 13 remind me 1 day before",
            now(),
        );
        assert_eq!(q.title, "Draft plan");
        assert_eq!(q.fields.start, Some(date(2026, 11, 6)));
        assert_eq!(q.fields.due.unwrap().to_string(), "2026-11-13");
        assert_eq!(q.fields.reminders[0].to_string(), "-PT24H");
        let q = parse("Call remind at 8am friday", now());
        assert_eq!(q.fields.reminders[0].to_string(), "2026-11-06T08:00");
    }

    #[test]
    fn quotes_and_spans() {
        let input = "Watch \"next monday\" film tomorrow";
        let q = parse(input, now());
        assert_eq!(q.title, "Watch \"next monday\" film");
        assert_eq!(q.fields.due.unwrap().to_string(), "2026-11-03");
        assert_eq!(&input[q.tokens[0].span.clone()], "tomorrow");
        // Only the first deadline counts.
        let q = parse("Move meeting from friday to monday", now());
        assert_eq!(q.fields.start, Some(date(2026, 11, 6)));
        assert_eq!(q.fields.due.unwrap().to_string(), "2026-11-09");
        let q = parse("today or tomorrow", now());
        assert_eq!(q.title, "or tomorrow");
    }
}
