// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Insights: cycle time, lead time and triage time.
//!
//! | Metric | From | To |
//! |---|---|---|
//! | Lead time | created | completed |
//! | Cycle time | started | completed |
//! | Triage time | created | triaged |
//!
//! Only completed issues count for lead and cycle time; an issue completed
//! without ever being started has no cycle time and is reported as
//! skipped rather than counted as zero. Results are grouped (by team,
//! assignee, label, project, estimate or completion week) and give the
//! count, mean and the 50th, 75th, 90th and 95th percentiles by the
//! nearest-rank method, all in hours.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};
use serde::{Deserialize, Serialize};

/// The lifecycle of one issue.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Lifecycle {
    /// The issue key.
    pub key: String,
    /// Its team.
    pub team: String,
    /// Its assignee.
    pub assignee: Option<String>,
    /// Its labels.
    pub labels: Vec<String>,
    /// Its project.
    pub project: Option<String>,
    /// Its estimate.
    pub estimate: Option<String>,
    /// When it was created.
    pub created_at: Option<Timestamp>,
    /// When it left triage.
    pub triaged_at: Option<Timestamp>,
    /// When it entered a started state.
    pub started_at: Option<Timestamp>,
    /// When it was completed.
    pub completed_at: Option<Timestamp>,
}

/// Which interval to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Created → completed.
    LeadTime,
    /// Started → completed.
    CycleTime,
    /// Created → triaged.
    TriageTime,
}

/// How to group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum GroupBy {
    /// One group.
    None,
    /// By team.
    Team,
    /// By assignee (`unassigned` when none).
    Assignee,
    /// By label; an issue with two labels counts in both.
    Label,
    /// By project.
    Project,
    /// By estimate.
    Estimate,
    /// By the ISO week of the end event, in the team's time zone.
    Week {
        /// IANA zone name.
        time_zone: String,
    },
}

/// Statistics for one group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// The group's name.
    pub group: String,
    /// Issues measured.
    pub count: usize,
    /// Mean, in hours.
    pub mean: f64,
    /// Median, in hours.
    pub p50: f64,
    /// 75th percentile, in hours.
    pub p75: f64,
    /// 90th percentile, in hours.
    pub p90: f64,
    /// 95th percentile, in hours.
    pub p95: f64,
}

/// The result of a query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The metric.
    pub metric: Metric,
    /// Groups, sorted by name.
    pub groups: Vec<Stats>,
    /// Issues without the needed timestamps (for example completed but
    /// never started), or with an end before the start.
    pub skipped: usize,
}

fn interval(issue: &Lifecycle, metric: Metric) -> Option<(f64, Timestamp)> {
    let (start, end) = match metric {
        Metric::LeadTime => (issue.created_at?, issue.completed_at?),
        Metric::CycleTime => (issue.started_at?, issue.completed_at?),
        Metric::TriageTime => (issue.created_at?, issue.triaged_at?),
    };
    let seconds = end.duration_since(start).as_secs_f64();
    (seconds >= 0.0).then_some((seconds / 3600.0, end))
}

/// Nearest rank: the smallest value with at least `p` percent at or below it.
fn percentile(sorted: &[f64], p: usize) -> f64 {
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted
        .get(rank - 1)
        .or_else(|| sorted.last())
        .copied()
        .unwrap_or(0.0)
}

/// A count as `f64`; counts here are far below 2^32.
fn count_f64(n: usize) -> f64 {
    u32::try_from(n).map_or(4_294_967_295.0, f64::from)
}

fn week(end: Timestamp, tz: &TimeZone) -> String {
    let date = end.to_zoned(tz.clone()).date();
    let iso = date.iso_week_date();
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// Computes a report.
#[must_use]
pub fn report(issues: &[Lifecycle], metric: Metric, group_by: &GroupBy) -> Report {
    let tz = match group_by {
        GroupBy::Week { time_zone } => TimeZone::get(time_zone).unwrap_or(TimeZone::UTC),
        _ => TimeZone::UTC,
    };
    let mut groups: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut skipped = 0;
    for issue in issues {
        let Some((hours, end)) = interval(issue, metric) else {
            skipped += 1;
            continue;
        };
        let names: Vec<String> = match group_by {
            GroupBy::None => vec!["all".to_owned()],
            GroupBy::Team => vec![issue.team.clone()],
            GroupBy::Assignee => vec![
                issue
                    .assignee
                    .clone()
                    .unwrap_or_else(|| "unassigned".to_owned()),
            ],
            GroupBy::Label if issue.labels.is_empty() => vec!["no label".to_owned()],
            GroupBy::Label => issue.labels.clone(),
            GroupBy::Project => vec![
                issue
                    .project
                    .clone()
                    .unwrap_or_else(|| "no project".to_owned()),
            ],
            GroupBy::Estimate => vec![
                issue
                    .estimate
                    .clone()
                    .unwrap_or_else(|| "no estimate".to_owned()),
            ],
            GroupBy::Week { .. } => vec![week(end, &tz)],
        };
        for name in names {
            groups.entry(name).or_default().push(hours);
        }
    }
    let groups = groups
        .into_iter()
        .map(|(group, mut values)| {
            values.sort_by(f64::total_cmp);
            let count = values.len();
            let mean = values.iter().sum::<f64>() / count_f64(count.max(1));
            Stats {
                group,
                count,
                mean,
                p50: percentile(&values, 50),
                p75: percentile(&values, 75),
                p90: percentile(&values, 90),
                p95: percentile(&values, 95),
            }
        })
        .collect();
    Report {
        metric,
        groups,
        skipped,
    }
}

/// The issues completed in the `weeks` full weeks before `now`, for the
/// default "last 12 weeks" window.
#[must_use]
pub fn window(issues: &[Lifecycle], now: Timestamp, weeks: i64) -> Vec<Lifecycle> {
    let since = now
        .checked_sub((weeks * 7 * 24).hours())
        .unwrap_or(Timestamp::MIN);
    issues
        .iter()
        .filter(|i| i.completed_at.is_some_and(|c| c >= since && c <= now))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hours(h: i64) -> Option<Timestamp> {
        Timestamp::from_second(1_793_260_800 + h * 3600).ok()
    }

    fn issue(key: &str, created: i64, started: Option<i64>, completed: Option<i64>) -> Lifecycle {
        Lifecycle {
            key: key.to_owned(),
            team: "ENG".to_owned(),
            created_at: hours(created),
            started_at: started.and_then(hours),
            completed_at: completed.and_then(hours),
            labels: vec!["bug".to_owned()],
            ..Lifecycle::default()
        }
    }

    #[test]
    fn lead_and_cycle_time() {
        let issues = vec![
            issue("A", 0, Some(10), Some(20)),
            issue("B", 0, Some(30), Some(40)),
            issue("C", 0, None, Some(50)),
            issue("D", 0, Some(5), None),
        ];
        let lead = report(&issues, Metric::LeadTime, &GroupBy::None);
        let all = lead.groups.first().cloned();
        assert_eq!(all.as_ref().map(|s| s.count), Some(3));
        assert_eq!(all.as_ref().map(|s| s.p50), Some(40.0));
        assert_eq!(all.as_ref().map(|s| s.mean.round()), Some(37.0));
        assert_eq!(lead.skipped, 1);
        let cycle = report(&issues, Metric::CycleTime, &GroupBy::Label);
        assert_eq!(
            cycle
                .groups
                .first()
                .map(|s| (s.group.as_str(), s.count, s.p95)),
            Some(("bug", 2, 10.0))
        );
        assert_eq!(cycle.skipped, 2);
    }

    #[test]
    fn nearest_rank_percentiles() {
        let values: Vec<f64> = (1..=20).map(f64::from).collect();
        assert!((percentile(&values, 50) - 10.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 95) - 19.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 100) - 20.0).abs() < f64::EPSILON);
        assert!(percentile(&[], 50).abs() < f64::EPSILON);
    }

    #[test]
    fn weeks_and_windows() {
        let issues = vec![
            issue("A", 0, Some(1), Some(2)),
            issue("B", -500, Some(-499), Some(-498)),
        ];
        let by_week = report(
            &issues,
            Metric::CycleTime,
            &GroupBy::Week {
                time_zone: "Europe/London".to_owned(),
            },
        );
        let names: Vec<&str> = by_week.groups.iter().map(|g| g.group.as_str()).collect();
        assert_eq!(names, ["2026-W41", "2026-W44"]);
        let now = hours(24).unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(window(&issues, now, 1).len(), 1);
        assert_eq!(window(&issues, now, 12).len(), 2);
    }
}
