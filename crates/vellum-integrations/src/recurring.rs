// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Recurring issues.
//!
//! A schedule holds an issue template, a rule (every N days, weeks on
//! chosen weekdays, months on a day, or years) and the team's time zone.
//! Each occurrence has a due date. The first issue is created when the
//! schedule is saved; each later one is created at **00:01 team time on
//! the day after the previous occurrence's due date**, so exactly one open
//! issue exists per period and it appears the morning the previous period
//! ends (brief §8).
//!
//! If Vellum was down for several periods, only the most recent missed
//! occurrence is created and the skipped ones are counted, so an outage
//! never floods a triage inbox (D81).

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};
use serde::{Deserialize, Serialize};

use crate::model::{IssueDraft, Source};

/// A day of the week, as stored in schedules.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Day {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl From<Day> for Weekday {
    fn from(day: Day) -> Self {
        match day {
            Day::Monday => Self::Monday,
            Day::Tuesday => Self::Tuesday,
            Day::Wednesday => Self::Wednesday,
            Day::Thursday => Self::Thursday,
            Day::Friday => Self::Friday,
            Day::Saturday => Self::Saturday,
            Day::Sunday => Self::Sunday,
        }
    }
}

/// How often a schedule repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Rule {
    /// Every `interval` days; with `weekdays_only`, every working day.
    Days {
        /// Days between occurrences, at least 1.
        interval: u16,
        /// Skip Saturdays and Sundays (the interval is then 1).
        weekdays_only: bool,
    },
    /// Every `interval` weeks on the given days.
    Weeks {
        /// Weeks between occurrences, at least 1.
        interval: u16,
        /// The weekdays; empty means the first due date's weekday.
        on: Vec<Day>,
    },
    /// Every `interval` months on a day of the month.
    Months {
        /// Months between occurrences, at least 1.
        interval: u16,
        /// 1–31 (clamped to the month's length) or -1 for the last day;
        /// `None` means the first due date's day.
        day: Option<i8>,
    },
    /// Every `interval` years on the first due date's month and day.
    Years {
        /// Years between occurrences, at least 1.
        interval: u16,
    },
}

/// When a schedule stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ends", content = "value", rename_all = "snake_case")]
pub enum End {
    /// Never.
    Never,
    /// After this many issues.
    After(u32),
    /// After this date (inclusive).
    On(Date),
}

/// A recurring issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// The schedule's ID.
    pub id: String,
    /// The issue to create; its `due` is set per occurrence.
    pub template: IssueDraft,
    /// The rule.
    pub rule: Rule,
    /// The team's IANA time zone, such as `Europe/London`.
    pub time_zone: String,
    /// The first occurrence's due date.
    pub first_due: Date,
    /// When it stops.
    pub end: End,
    /// Issues created so far.
    pub created: u32,
    /// The due date of the last issue created.
    pub last_due: Option<Date>,
    /// Paused schedules create nothing.
    pub paused: bool,
}

/// Why a schedule cannot run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    /// An interval of zero, or a day of month outside 1–31 and -1.
    #[error("invalid rule: {0}")]
    InvalidRule(&'static str),
    /// The time zone is not in the tz database.
    #[error("unknown time zone `{0}`")]
    UnknownTimeZone(String),
}

/// Caps iteration so a pathological rule cannot spin.
const MAX_STEPS: usize = 100_000;

fn clamp_day(year: i16, month: i8, day: i8) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    let last = first.days_in_month();
    let day = if day < 0 { last } else { day.min(last) };
    Date::new(year, month, day).ok()
}

impl Rule {
    /// Checks the rule's numbers.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::InvalidRule`] for a zero interval or bad day.
    pub fn validate(&self) -> Result<(), ScheduleError> {
        let interval = match self {
            Self::Days { interval, .. }
            | Self::Weeks { interval, .. }
            | Self::Months { interval, .. }
            | Self::Years { interval } => *interval,
        };
        if interval == 0 {
            return Err(ScheduleError::InvalidRule(
                "interval must be at least 1",
            ));
        }
        if let Self::Months { day: Some(day), .. } = self
            && !(*day == -1 || (1..=31).contains(day))
        {
            return Err(ScheduleError::InvalidRule("day must be 1–31 or -1"));
        }
        Ok(())
    }

    /// The first occurrence strictly after `after`, given the first due date.
    #[must_use]
    pub fn next_after(&self, first: Date, after: Option<Date>) -> Option<Date> {
        let Some(after) = after else {
            return Some(first);
        };
        self.occurrences(first).take(MAX_STEPS).find(|d| *d > after)
    }

    /// Every occurrence from `first` on, in order.
    pub fn occurrences(
        &self,
        first: Date,
    ) -> Box<dyn Iterator<Item = Date> + '_> {
        match self {
            Self::Days {
                weekdays_only: true,
                ..
            } => Box::new(first.series(1.day()).filter(|d| {
                !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
            })),
            Self::Days { interval, .. } => {
                Box::new(first.series(i64::from(*interval).days()))
            }
            Self::Weeks { interval, on } => {
                let mut days: Vec<Weekday> = if on.is_empty() {
                    vec![first.weekday()]
                } else {
                    on.iter().copied().map(Weekday::from).collect()
                };
                days.sort_by_key(|d| d.to_monday_zero_offset());
                days.dedup();
                let monday = first
                    .checked_sub(
                        i64::from(first.weekday().to_monday_zero_offset())
                            .days(),
                    )
                    .unwrap_or(first);
                Box::new(
                    monday
                        .series(i64::from(*interval).weeks())
                        .flat_map(move |week| {
                            days.clone().into_iter().filter_map(move |day| {
                                week.checked_add(
                                    i64::from(day.to_monday_zero_offset())
                                        .days(),
                                )
                                .ok()
                            })
                        })
                        .filter(move |d| *d >= first),
                )
            }
            Self::Months { interval, day } => {
                let day = day.unwrap_or_else(|| first.day());
                let interval = i32::from(*interval);
                Box::new(
                    (0..)
                        .map_while(move |k: i32| {
                            let months = i32::from(first.year()) * 12
                                + i32::from(first.month())
                                - 1
                                + k * interval;
                            let year =
                                i16::try_from(months.div_euclid(12)).ok()?;
                            let month =
                                i8::try_from(months.rem_euclid(12) + 1).ok()?;
                            clamp_day(year, month, day)
                        })
                        .filter(move |d| *d >= first),
                )
            }
            Self::Years { interval } => {
                let interval = i32::from(*interval);
                Box::new((0..).map_while(move |k: i32| {
                    let year =
                        i16::try_from(i32::from(first.year()) + k * interval)
                            .ok()?;
                    clamp_day(year, first.month(), first.day())
                }))
            }
        }
    }
}

/// An issue a schedule wants created now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    /// The draft, with `due` and the recurring source set.
    pub draft: IssueDraft,
    /// Its due date; store it as the schedule's `last_due`.
    pub due: Date,
    /// Occurrences skipped because Vellum was down.
    pub skipped: u32,
}

impl Schedule {
    fn ended(&self, due: Date, created: u32) -> bool {
        match self.end {
            End::Never => false,
            End::After(count) => created >= count,
            End::On(last) => due > last,
        }
    }

    /// When the occurrence after `previous_due` is created: 00:01 team time
    /// the next day.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::UnknownTimeZone`] for a bad zone name.
    pub fn creation_time(
        &self,
        previous_due: Date,
    ) -> Result<Timestamp, ScheduleError> {
        let tz = TimeZone::get(&self.time_zone).map_err(|_| {
            ScheduleError::UnknownTimeZone(self.time_zone.clone())
        })?;
        let day = previous_due.tomorrow().unwrap_or(previous_due);
        day.to_datetime(Time::constant(0, 1, 0, 0))
            .to_zoned(tz)
            .map(|z| z.timestamp())
            .map_err(|_| ScheduleError::UnknownTimeZone(self.time_zone.clone()))
    }

    /// What to create at `now`, if anything. The caller creates the issue,
    /// then stores `created + 1` and `last_due = due`.
    ///
    /// # Errors
    ///
    /// [`ScheduleError`] for an invalid rule or time zone.
    pub fn due_now(
        &self,
        now: Timestamp,
    ) -> Result<Option<Occurrence>, ScheduleError> {
        self.rule.validate()?;
        if self.paused {
            return Ok(None);
        }
        let mut previous = self.last_due;
        let mut pick: Option<Date> = None;
        let mut skipped = 0_u32;
        for created in (self.created..).take(MAX_STEPS) {
            let Some(next) = self.rule.next_after(self.first_due, previous)
            else {
                break;
            };
            if self.ended(next, created) {
                break;
            }
            let ready = match previous {
                None => true,
                Some(prev) => self.creation_time(prev)? <= now,
            };
            if !ready {
                break;
            }
            if pick.is_some() {
                skipped += 1;
            }
            pick = Some(next);
            previous = Some(next);
            if self.last_due.is_none() {
                // The first issue is created on save; never catch up past it.
                break;
            }
        }
        Ok(pick.map(|due| {
            let mut draft = self.template.clone();
            draft.due = Some(due);
            draft.source = Source::Recurring {
                schedule_id: self.id.clone(),
            };
            Occurrence {
                draft,
                due,
                skipped,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn dates(rule: &Rule, first: Date, n: usize) -> Vec<String> {
        rule.occurrences(first)
            .take(n)
            .map(|d| d.to_string())
            .collect()
    }

    #[test]
    fn rules() {
        let first = date(2026, 1, 30); // a Friday
        assert_eq!(
            dates(
                &Rule::Days {
                    interval: 3,
                    weekdays_only: false
                },
                first,
                3
            ),
            ["2026-01-30", "2026-02-02", "2026-02-05"]
        );
        assert_eq!(
            dates(
                &Rule::Days {
                    interval: 1,
                    weekdays_only: true
                },
                first,
                3
            ),
            ["2026-01-30", "2026-02-02", "2026-02-03"]
        );
        assert_eq!(
            dates(
                &Rule::Weeks {
                    interval: 2,
                    on: vec![Day::Monday, Day::Friday]
                },
                first,
                4
            ),
            ["2026-01-30", "2026-02-09", "2026-02-13", "2026-02-23"]
        );
        assert_eq!(
            dates(
                &Rule::Months {
                    interval: 1,
                    day: None
                },
                first,
                3
            ),
            ["2026-01-30", "2026-02-28", "2026-03-30"]
        );
        assert_eq!(
            dates(
                &Rule::Months {
                    interval: 1,
                    day: Some(-1)
                },
                first,
                3
            ),
            ["2026-01-31", "2026-02-28", "2026-03-31"]
        );
        assert_eq!(
            dates(&Rule::Years { interval: 1 }, date(2028, 2, 29), 2),
            ["2028-02-29", "2029-02-28"]
        );
        assert!(
            Rule::Days {
                interval: 0,
                weekdays_only: false
            }
            .validate()
            .is_err()
        );
        assert!(
            Rule::Months {
                interval: 1,
                day: Some(32)
            }
            .validate()
            .is_err()
        );
    }

    fn schedule() -> Schedule {
        Schedule {
            id: "s1".to_owned(),
            template: IssueDraft::new("OPS", "Rotate on-call", Source::App),
            rule: Rule::Weeks {
                interval: 1,
                on: Vec::new(),
            },
            time_zone: "America/New_York".to_owned(),
            first_due: date(2026, 11, 2),
            end: End::Never,
            created: 0,
            last_due: None,
            paused: false,
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap_or(Timestamp::UNIX_EPOCH)
    }

    #[test]
    fn first_issue_is_created_on_save() -> Result<(), ScheduleError> {
        let occurrence = schedule().due_now(at("2026-10-29T12:00:00Z"))?;
        assert_eq!(occurrence.as_ref().map(|o| o.due), Some(date(2026, 11, 2)));
        let draft = occurrence.map(|o| o.draft);
        assert_eq!(draft.as_ref().and_then(|d| d.due), Some(date(2026, 11, 2)));
        assert_eq!(
            draft.map(|d| d.source),
            Some(Source::Recurring {
                schedule_id: "s1".to_owned()
            })
        );
        Ok(())
    }

    #[test]
    fn next_issue_appears_at_0001_team_time_after_the_due_date()
    -> Result<(), ScheduleError> {
        let mut s = schedule();
        s.created = 1;
        s.last_due = Some(date(2026, 11, 2));
        // 00:01 on 3 November in New York is 05:01 UTC (EST).
        assert_eq!(
            s.creation_time(date(2026, 11, 2))?,
            at("2026-11-03T05:01:00Z")
        );
        assert_eq!(s.due_now(at("2026-11-03T05:00:59Z"))?, None);
        let occurrence = s.due_now(at("2026-11-03T05:01:00Z"))?;
        assert_eq!(
            occurrence.map(|o| (o.due, o.skipped)),
            Some((date(2026, 11, 9), 0))
        );
        Ok(())
    }

    #[test]
    fn outages_create_only_the_latest_missed_occurrence()
    -> Result<(), ScheduleError> {
        let mut s = schedule();
        s.created = 1;
        s.last_due = Some(date(2026, 11, 2));
        let occurrence = s.due_now(at("2026-11-20T12:00:00Z"))?;
        // Due 9th (created 3rd) and 16th (created 10th) are ready; the 23rd's
        // creation time (17th) has passed too.
        assert_eq!(
            occurrence.map(|o| (o.due, o.skipped)),
            Some((date(2026, 11, 23), 2))
        );
        Ok(())
    }

    #[test]
    fn ends_pauses_and_zones() -> Result<(), ScheduleError> {
        let mut s = schedule();
        s.created = 1;
        s.last_due = Some(date(2026, 11, 2));
        s.end = End::After(1);
        assert_eq!(s.due_now(at("2027-01-01T00:00:00Z"))?, None);
        s.end = End::On(date(2026, 11, 5));
        assert_eq!(s.due_now(at("2027-01-01T00:00:00Z"))?, None);
        s.end = End::Never;
        s.paused = true;
        assert_eq!(s.due_now(at("2027-01-01T00:00:00Z"))?, None);
        s.paused = false;
        s.time_zone = "Mars/Olympus".to_owned();
        assert!(s.due_now(at("2027-01-01T00:00:00Z")).is_err());
        Ok(())
    }
}
