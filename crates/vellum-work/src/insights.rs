// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Insights: how long work takes and how much gets done (R-WK-15).
//!
//! Everything comes from the lifecycle timestamps every issue already
//! carries (`created_at`, `triaged_at`, `started_at`, `completed_at`), so
//! nothing extra is tracked. A query picks the issues (the same filter as
//! views), a measure, an optional breakdown ("pivot") and a date range,
//! and gets percentiles per breakdown value, optionally as a weekly or
//! monthly series.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::{Date, Duration, Month, OffsetDateTime, Time, Weekday};

use crate::error::{Result, WorkError};
use crate::filter::Filter;
use crate::graph::{Ctx, WorkGraph};
use crate::id::UserRef;
use crate::issue::Issue;

/// The longest range a query may cover.
pub const MAX_DAYS: i64 = 2 * 366;

/// What to measure.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    /// Created to completed, for issues completed in the range.
    #[default]
    LeadTime,
    /// Started to completed, for issues completed in the range that were
    /// ever started.
    CycleTime,
    /// Created to the triage decision, for issues triaged in the range.
    TriageTime,
    /// How many issues were completed in the range.
    Throughput,
    /// How many issues were created in the range.
    Created,
}

impl Measure {
    /// Whether the measure is a duration (with percentiles) or a count.
    #[must_use]
    pub const fn is_duration(self) -> bool {
        matches!(self, Self::LeadTime | Self::CycleTime | Self::TriageTime)
    }

    /// The moment that places an issue in the range, and the duration it
    /// contributes, if any.
    fn sample(
        self,
        issue: &Issue,
    ) -> Option<(OffsetDateTime, Option<Duration>)> {
        match self {
            Self::LeadTime => issue
                .completed_at
                .map(|done| (done, Some(done - issue.created_at))),
            Self::CycleTime => issue
                .completed_at
                .zip(issue.started_at)
                .map(|(done, start)| (done, Some(done - start))),
            Self::TriageTime => {
                issue.triaged_at.map(|at| (at, Some(at - issue.created_at)))
            }
            Self::Throughput => issue.completed_at.map(|done| (done, None)),
            Self::Created => Some((issue.created_at, None)),
        }
    }
}

/// How to break results down.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Pivot {
    /// One bucket with everything.
    #[default]
    None,
    /// By team.
    Team,
    /// By current assignee.
    Assignee,
    /// By priority.
    Priority,
    /// By label; an issue with two labels counts in both.
    Label,
    /// By project.
    Project,
    /// By cycle.
    Cycle,
    /// By estimate.
    Estimate,
}

impl Pivot {
    /// The bucket keys of an issue; `None` is the "no value" bucket.
    fn keys(self, issue: &Issue) -> Vec<Option<String>> {
        match self {
            Self::None => vec![None],
            Self::Team => vec![Some(issue.team.to_string())],
            Self::Assignee => {
                vec![issue.assignee.as_ref().map(ToString::to_string)]
            }
            Self::Priority => vec![Some(issue.priority.to_string())],
            Self::Label if issue.labels.is_empty() => vec![None],
            Self::Label => issue.labels.iter().cloned().map(Some).collect(),
            Self::Project => vec![issue.project.map(|p| p.to_string())],
            Self::Cycle => vec![issue.cycle.as_ref().map(ToString::to_string)],
            Self::Estimate => vec![issue.estimate.map(|e| e.to_string())],
        }
    }
}

/// The width of each point of a series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interval {
    /// Monday to Sunday.
    Week,
    /// Calendar months.
    Month,
}

impl Interval {
    /// The start of the interval holding `day`.
    fn start(self, day: Date) -> Date {
        match self {
            Self::Week => {
                day - Duration::days(i64::from(
                    day.weekday().number_days_from_monday(),
                ))
            }
            Self::Month => day.replace_day(1).unwrap_or(day),
        }
    }

    /// The start of the next interval.
    fn next(self, start: Date) -> Date {
        match self {
            Self::Week => start + Duration::weeks(1),
            Self::Month => {
                let (year, month) = if start.month() == Month::December {
                    (start.year() + 1, Month::January)
                } else {
                    (start.year(), start.month().next())
                };
                Date::from_calendar_date(year, month, 1).unwrap_or(start)
            }
        }
    }
}

/// An insights question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    /// Which issues; empty means all.
    #[serde(default)]
    pub filter: Filter,
    /// What to measure.
    #[serde(default)]
    pub measure: Measure,
    /// How to break it down.
    #[serde(default)]
    pub pivot: Pivot,
    /// First day of the range.
    pub from: Date,
    /// Day after the last day of the range.
    pub to: Date,
    /// Split each bucket into weeks or months.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<Interval>,
}

impl Query {
    /// Checks the range and filter.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] for an empty or too long range or a
    /// bad filter.
    pub fn check(&self) -> Result<()> {
        let invalid = |reason| WorkError::Invalid {
            what: "insights range",
            value: format!("{}..{}", self.from, self.to),
            reason,
        };
        if self.to <= self.from {
            return Err(invalid("must end after it starts"));
        }
        if (self.to - self.from).whole_days() > MAX_DAYS {
            return Err(invalid("must be at most two years"));
        }
        self.filter.check().map_err(|_| WorkError::Invalid {
            what: "insights filter",
            value: String::new(),
            reason: "is not a valid filter",
        })
    }
}

/// Summary of a set of samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// How many issues.
    pub count: u32,
    /// Median, in hours (durations only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_hours: Option<f64>,
    /// 85th percentile, in hours: "most work finishes within".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p85_hours: Option<f64>,
    /// 95th percentile, in hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_hours: Option<f64>,
    /// Mean, in hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_hours: Option<f64>,
}

impl Stats {
    fn of(count: usize, durations: &mut [Duration]) -> Self {
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        if durations.is_empty() {
            return Self {
                count,
                ..Self::default()
            };
        }
        durations.sort_unstable();
        let hours = |d: Duration| d.as_seconds_f64() / 3600.0;
        let total: Duration = durations.iter().copied().sum();
        #[allow(
            clippy::cast_precision_loss,
            reason = "counts are far below 2^52"
        )]
        let mean = hours(total) / durations.len() as f64;
        Self {
            count,
            p50_hours: percentile(durations, 50).map(hours),
            p85_hours: percentile(durations, 85).map(hours),
            p95_hours: percentile(durations, 95).map(hours),
            mean_hours: Some(mean),
        }
    }
}

/// The nearest-rank percentile of sorted samples.
fn percentile(sorted: &[Duration], p: usize) -> Option<Duration> {
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

/// One point of a series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// First day of the week or month.
    pub start: Date,
    /// The stats for that interval.
    pub stats: Stats,
}

/// One breakdown value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bucket {
    /// The value (`ENG`, `dev`, `urgent`, `gateway`); `None` is "none".
    pub key: Option<String>,
    /// Over the whole range.
    pub overall: Stats,
    /// Per interval, when asked for; every interval is present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<Point>,
}

/// The answer to a [`Query`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Insights {
    /// Everything together.
    pub overall: Stats,
    /// Per breakdown value, largest first.
    pub buckets: Vec<Bucket>,
}

type Samples = Vec<(Date, Option<Duration>)>;

/// Answers an insights query for a viewer.
///
/// # Errors
///
/// Returns the [`Query::check`] error.
pub fn run(
    graph: &WorkGraph,
    query: &Query,
    ctx: &Ctx,
    me: Option<&UserRef>,
) -> Result<Insights> {
    query.check()?;
    let ectx = graph.eval_context(ctx, me);
    let from = query.from.with_time(Time::MIDNIGHT).assume_utc();
    let to = query.to.with_time(Time::MIDNIGHT).assume_utc();
    let mut by_key: BTreeMap<Option<String>, Samples> = BTreeMap::new();
    let mut all: Samples = Vec::new();
    for issue in graph.issues() {
        let Some((at, duration)) = query.measure.sample(issue) else {
            continue;
        };
        if at < from || at >= to || !query.filter.matches(issue, &ectx) {
            continue;
        }
        let day = at.date();
        all.push((day, duration));
        for key in query.pivot.keys(issue) {
            by_key.entry(key).or_default().push((day, duration));
        }
    }
    let mut buckets: Vec<Bucket> = by_key
        .into_iter()
        .map(|(key, samples)| Bucket {
            key,
            overall: summarise(&samples),
            series: query
                .interval
                .map(|i| series(&samples, i, query.from, query.to))
                .unwrap_or_default(),
        })
        .collect();
    buckets.sort_by(|a, b| {
        b.overall
            .count
            .cmp(&a.overall.count)
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(Insights {
        overall: summarise(&all),
        buckets,
    })
}

fn summarise(samples: &Samples) -> Stats {
    let mut durations: Vec<Duration> =
        samples.iter().filter_map(|(_, d)| *d).collect();
    Stats::of(samples.len(), &mut durations)
}

fn series(
    samples: &Samples,
    interval: Interval,
    from: Date,
    to: Date,
) -> Vec<Point> {
    let mut out = Vec::new();
    let mut start = interval.start(from);
    while start < to {
        let end = interval.next(start);
        let inside: Samples = samples
            .iter()
            .filter(|(day, _)| *day >= start && *day < end)
            .copied()
            .collect();
        out.push(Point {
            start,
            stats: summarise(&inside),
        });
        if end <= start {
            break;
        }
        start = end;
    }
    out
}

/// The Monday of the week holding `day`; handy for "this week" ranges.
#[must_use]
pub fn week_of(day: Date) -> Date {
    let mut d = day;
    while d.weekday() != Weekday::Monday {
        d = d.previous_day().unwrap_or(d);
    }
    d
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    fn hours(h: i64) -> Duration {
        Duration::hours(h)
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut d: Vec<Duration> = (1..=20).map(hours).collect();
        let s = Stats::of(d.len(), &mut d);
        assert_eq!(s.count, 20);
        assert_eq!(s.p50_hours, Some(10.0));
        assert_eq!(s.p85_hours, Some(17.0));
        assert_eq!(s.p95_hours, Some(19.0));
        assert_eq!(s.mean_hours, Some(10.5));
        let mut one = vec![hours(5)];
        assert_eq!(Stats::of(1, &mut one).p95_hours, Some(5.0));
        assert_eq!(Stats::of(0, &mut []).p50_hours, None);
    }

    #[test]
    fn intervals_start_on_mondays_and_month_starts() {
        assert_eq!(
            Interval::Week.start(date!(2026 - 11 - 05)),
            date!(2026 - 11 - 02)
        );
        assert_eq!(
            Interval::Month.start(date!(2026 - 11 - 05)),
            date!(2026 - 11 - 01)
        );
        assert_eq!(
            Interval::Month.next(date!(2026 - 12 - 01)),
            date!(2027 - 01 - 01)
        );
        assert_eq!(week_of(date!(2026 - 11 - 08)), date!(2026 - 11 - 02));
    }

    #[test]
    fn ranges_are_checked() {
        let q = |from, to| Query {
            filter: Filter::default(),
            measure: Measure::LeadTime,
            pivot: Pivot::None,
            from,
            to,
            interval: None,
        };
        assert!(
            q(date!(2026 - 11 - 01), date!(2026 - 12 - 01))
                .check()
                .is_ok()
        );
        assert!(
            q(date!(2026 - 11 - 01), date!(2026 - 11 - 01))
                .check()
                .is_err()
        );
        assert!(
            q(date!(2020 - 01 - 01), date!(2026 - 01 - 01))
                .check()
                .is_err()
        );
    }
}
