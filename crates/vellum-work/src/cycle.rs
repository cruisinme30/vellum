// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Cycles that run themselves (D16).
//!
//! Per team: duration, start day, optional cooldown and the number of
//! future cycles kept pre-created. When a cycle ends, unfinished issues roll
//! into the next one; started or completed issues with no cycle join the
//! current one. A burn-up point is recorded each day; velocity is the mean
//! completed points of the last three finished cycles.
//!
//! Cycles are stored as `teams/ENG/cycles/2027-C04.yaml`. Dates are in the
//! team's time zone; the caller passes the team's `today`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use time::{Date, Duration, Weekday};

use crate::error::WorkError;
use crate::issue::Issue;
use crate::team::TeamKey;
use crate::workflow::Category;

/// A cycle's key within its team: `<year>-C<nn>`, numbered by start date
/// within the year, such as `2027-C04`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CycleKey(String);

impl CycleKey {
    /// Parses `2027-C04`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] unless the text is a four-digit year,
    /// `-C` and a number from 01 to 99.
    pub fn parse(text: &str) -> Result<Self, WorkError> {
        let ok = text.len() == 8
            && text
                .get(..4)
                .is_some_and(|y| y.bytes().all(|b| b.is_ascii_digit()))
            && text.get(4..6) == Some("-C")
            && text
                .get(6..)
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()) && n != "00");
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(WorkError::Invalid {
                what: "cycle key",
                value: text.to_owned(),
                reason: "must be YYYY-Cnn, such as 2027-C04",
            })
        }
    }

    fn new(year: i32, number: usize) -> Result<Self, WorkError> {
        Self::parse(&format!("{year:04}-C{number:02}"))
    }

    /// The key as written in files.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CycleKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CycleKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// The day of the week cycles start on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartDay {
    /// Monday.
    #[default]
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

impl StartDay {
    const fn weekday(self) -> Weekday {
        match self {
            Self::Monday => Weekday::Monday,
            Self::Tuesday => Weekday::Tuesday,
            Self::Wednesday => Weekday::Wednesday,
            Self::Thursday => Weekday::Thursday,
            Self::Friday => Weekday::Friday,
            Self::Saturday => Weekday::Saturday,
            Self::Sunday => Weekday::Sunday,
        }
    }
}

/// A team's cycle settings (`cycles:` in `team.yaml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleSettings {
    /// Length of a cycle in weeks, 1 to 8.
    pub duration_weeks: u8,
    /// The day cycles start on.
    #[serde(default)]
    pub start_day: StartDay,
    /// Weeks of cooldown between cycles, 0 to 4.
    #[serde(default)]
    pub cooldown_weeks: u8,
    /// Future cycles kept pre-created, 1 to 15.
    #[serde(default = "three")]
    pub upcoming: u8,
    /// Roll unfinished issues into the next cycle when a cycle ends.
    #[serde(default = "yes")]
    pub auto_rollover: bool,
    /// Add issues that start, with no cycle, to the current cycle.
    #[serde(default = "yes")]
    pub auto_add_started: bool,
    /// Add issues that complete, with no cycle, to the current cycle.
    #[serde(default = "yes")]
    pub auto_add_completed: bool,
}

const fn three() -> u8 {
    3
}

const fn yes() -> bool {
    true
}

impl CycleSettings {
    /// Two-week cycles starting Monday, no cooldown, three upcoming.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            duration_weeks: 2,
            start_day: StartDay::Monday,
            cooldown_weeks: 0,
            upcoming: 3,
            auto_rollover: true,
            auto_add_started: true,
            auto_add_completed: true,
        }
    }

    /// Checks the ranges.
    ///
    /// # Errors
    ///
    /// Returns which setting is out of range.
    pub fn check(&self) -> Result<(), &'static str> {
        if !(1..=8).contains(&self.duration_weeks) {
            return Err("cycle duration must be 1 to 8 weeks");
        }
        if self.cooldown_weeks > 4 {
            return Err("cooldown must be 0 to 4 weeks");
        }
        if !(1..=15).contains(&self.upcoming) {
            return Err("upcoming cycles must be 1 to 15");
        }
        Ok(())
    }
}

/// One day's burn-up point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurnPoint {
    /// The day.
    pub date: Date,
    /// Points in the cycle (canceled and duplicate issues excluded).
    pub scope: u32,
    /// Points in started issues.
    pub started: u32,
    /// Points in completed issues.
    pub completed: u32,
    /// Issues in the cycle.
    pub scope_issues: u32,
    /// Completed issues.
    pub completed_issues: u32,
}

/// A cycle, stored as `teams/KEY/cycles/<key>.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cycle {
    /// The cycle key, unique within the team.
    pub key: CycleKey,
    /// The owning team.
    pub team: TeamKey,
    /// An optional name, such as "Replay hardening".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The first day.
    pub starts_on: Date,
    /// The day after the last day.
    pub ends_on: Date,
    /// Daily burn-up points, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub burn_up: Vec<BurnPoint>,
}

impl Cycle {
    /// The path of the cycle's file.
    #[must_use]
    pub fn path(&self) -> String {
        format!("teams/{}/cycles/{}.yaml", self.team, self.key)
    }

    /// Whether `today` falls inside the cycle.
    #[must_use]
    pub fn contains(&self, today: Date) -> bool {
        self.starts_on <= today && today < self.ends_on
    }

    /// Whether the cycle is over on `today`.
    #[must_use]
    pub fn is_past(&self, today: Date) -> bool {
        self.ends_on <= today
    }

    /// Completed points at the last recorded point.
    #[must_use]
    pub fn completed_points(&self) -> u32 {
        self.burn_up.last().map_or(0, |p| p.completed)
    }

    /// Records a burn-up point, replacing any point for the same day.
    pub fn record(&mut self, point: BurnPoint) {
        self.burn_up.retain(|p| p.date != point.date);
        self.burn_up.push(point);
        self.burn_up.sort_by_key(|p| p.date);
    }

    /// Parses a cycle file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the YAML is malformed or the dates
    /// are out of order.
    pub fn from_yaml(path: &str, text: &str) -> Result<Self, WorkError> {
        let err = |reason: String| WorkError::File {
            path: path.to_owned(),
            reason,
        };
        let cycle: Self = serde_saphyr::from_str(text).map_err(|e| err(e.to_string()))?;
        if cycle.ends_on <= cycle.starts_on {
            return Err(err("ends_on must be after starts_on".to_owned()));
        }
        Ok(cycle)
    }

    /// Serialises the cycle.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_yaml(&self) -> Result<String, WorkError> {
        serde_saphyr::to_string(self).map_err(|e| WorkError::File {
            path: self.path(),
            reason: e.to_string(),
        })
    }
}

/// The cycle containing `today`, if any (none during a cooldown).
#[must_use]
pub fn current(cycles: &[Cycle], today: Date) -> Option<&Cycle> {
    cycles.iter().find(|c| c.contains(today))
}

/// The first cycle that starts after `today`.
#[must_use]
pub fn next(cycles: &[Cycle], today: Date) -> Option<&Cycle> {
    cycles
        .iter()
        .filter(|c| c.starts_on > today)
        .min_by_key(|c| c.starts_on)
}

/// The cycles to create so that the current cycle (or, in a cooldown, the
/// next one) and `upcoming` future cycles exist.
///
/// The first cycle a team ever has starts on the most recent start day on
/// or before `today`. Each later cycle starts after the previous one ends
/// plus the cooldown.
///
/// # Errors
///
/// Returns [`WorkError::Invalid`] if the dates leave the supported range.
pub fn plan(
    team: &TeamKey,
    settings: &CycleSettings,
    existing: &[Cycle],
    today: Date,
) -> Result<Vec<Cycle>, WorkError> {
    let overflow = || WorkError::Invalid {
        what: "cycle date",
        value: today.to_string(),
        reason: "out of range",
    };
    let length = Duration::weeks(i64::from(settings.duration_weeks));
    let gap = Duration::weeks(i64::from(settings.cooldown_weeks));
    let mut last_end = existing.iter().map(|c| c.ends_on).max();
    let mut years: Vec<i32> = existing.iter().map(|c| c.starts_on.year()).collect();
    let live = |end: Date| end > today;
    let mut live_count = existing.iter().filter(|c| live(c.ends_on)).count();
    let wanted = usize::from(settings.upcoming) + 1;
    let mut out = Vec::new();
    while live_count < wanted {
        let starts_on = if let Some(end) = last_end {
            let mut start = end.checked_add(gap).ok_or_else(overflow)?;
            // A team that paused cycles restarts at the latest start
            // day on or before today rather than creating past cycles.
            while start.checked_add(length).ok_or_else(overflow)? <= today {
                start = start.checked_add(length + gap).ok_or_else(overflow)?;
            }
            start
        } else {
            let back = i64::from(today.weekday().number_days_from_monday())
                - i64::from(settings.start_day.weekday().number_days_from_monday());
            today
                .checked_sub(Duration::days(back.rem_euclid(7)))
                .ok_or_else(overflow)?
        };
        let ends_on = starts_on.checked_add(length).ok_or_else(overflow)?;
        let year = starts_on.year();
        years.push(year);
        let number = years.iter().filter(|y| **y == year).count();
        out.push(Cycle {
            key: CycleKey::new(year, number)?,
            team: team.clone(),
            name: None,
            starts_on,
            ends_on,
            burn_up: Vec::new(),
        });
        last_end = Some(ends_on);
        live_count += usize::from(live(ends_on));
    }
    Ok(out)
}

/// What happens to an issue in a cycle that has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rollover {
    /// The issue stays in the finished cycle (it is closed).
    Keep,
    /// The issue moves to the next cycle (it is unfinished).
    MoveToNext,
    /// The issue leaves the cycle (it went back to backlog or triage).
    Remove,
}

/// The rollover rule: closed issues stay; unstarted and started issues
/// move on; backlog and triage issues leave the cycle.
#[must_use]
pub const fn rollover_action(category: Category) -> Rollover {
    match category {
        Category::Completed | Category::Canceled | Category::Duplicate => Rollover::Keep,
        Category::Unstarted | Category::Started => Rollover::MoveToNext,
        Category::Triage | Category::Backlog => Rollover::Remove,
    }
}

/// Measures a cycle's issues for a burn-up point.
///
/// `issues` are the issues whose `cycle` is this cycle, each with its
/// category; an unestimated issue counts `unestimated_points`.
pub fn snapshot<'a>(
    date: Date,
    issues: impl IntoIterator<Item = (&'a Issue, Category)>,
    unestimated_points: u32,
) -> BurnPoint {
    let mut p = BurnPoint {
        date,
        scope: 0,
        started: 0,
        completed: 0,
        scope_issues: 0,
        completed_issues: 0,
    };
    for (issue, category) in issues {
        if matches!(category, Category::Canceled | Category::Duplicate) {
            continue;
        }
        let points = issue
            .estimate
            .map_or(unestimated_points, super::scale::Estimate::points);
        p.scope = p.scope.saturating_add(points);
        p.scope_issues = p.scope_issues.saturating_add(1);
        match category {
            Category::Started => p.started = p.started.saturating_add(points),
            Category::Completed => {
                p.completed = p.completed.saturating_add(points);
                p.completed_issues = p.completed_issues.saturating_add(1);
            }
            _ => {}
        }
    }
    p
}

/// Mean completed points of the last three cycles that ended by `today`.
#[must_use]
pub fn velocity(cycles: &[Cycle], today: Date) -> Option<f64> {
    let mut past: Vec<&Cycle> = cycles.iter().filter(|c| c.is_past(today)).collect();
    past.sort_by_key(|c| std::cmp::Reverse(c.ends_on));
    let last: Vec<u32> = past.iter().take(3).map(|c| c.completed_points()).collect();
    if last.is_empty() {
        return None;
    }
    let sum: u32 = last.iter().sum();
    #[allow(clippy::cast_precision_loss, reason = "at most three small counts")]
    Some(f64::from(sum) / last.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    fn eng() -> TeamKey {
        TeamKey::parse("ENG").unwrap()
    }

    #[test]
    fn keys_are_validated() {
        assert!(CycleKey::parse("2027-C04").is_ok());
        for bad in ["2027-C4", "2027-C00", "27-C04", "2027C004", "2027-c04"] {
            assert!(CycleKey::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_first_plan_starts_on_the_last_start_day() {
        let s = CycleSettings::standard();
        // 2026-10-29 is a Thursday.
        let cycles = plan(&eng(), &s, &[], date!(2026 - 10 - 29)).unwrap();
        assert_eq!(cycles.len(), 4);
        assert_eq!(cycles[0].starts_on, date!(2026 - 10 - 26));
        assert_eq!(cycles[0].ends_on, date!(2026 - 11 - 09));
        assert_eq!(cycles[0].key.as_str(), "2026-C01");
        assert_eq!(cycles[3].starts_on, date!(2026 - 12 - 07));
        assert_eq!(cycles[3].key.as_str(), "2026-C04");
        assert_eq!(
            current(&cycles, date!(2026 - 10 - 29)).unwrap().key,
            cycles[0].key
        );
        assert_eq!(
            next(&cycles, date!(2026 - 10 - 29)).unwrap().key,
            cycles[1].key
        );
        // Planning again creates nothing.
        assert!(
            plan(&eng(), &s, &cycles, date!(2026 - 10 - 29))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn plans_top_up_with_cooldown_and_year_numbering() {
        let s = CycleSettings {
            cooldown_weeks: 1,
            upcoming: 1,
            ..CycleSettings::standard()
        };
        let first = plan(&eng(), &s, &[], date!(2026 - 12 - 01)).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[1].starts_on, date!(2026 - 12 - 21));
        let mut all = first;
        let more = plan(&eng(), &s, &all, date!(2027 - 01 - 06)).unwrap();
        assert_eq!(more.len(), 2);
        assert_eq!(more[0].starts_on, date!(2027 - 01 - 11));
        assert_eq!(more[0].key.as_str(), "2027-C01");
        all.extend(more);
        assert!(current(&all, date!(2027 - 01 - 06)).is_none(), "cooldown");
    }

    #[test]
    fn plans_skip_ahead_after_a_pause() {
        let s = CycleSettings::standard();
        let old = plan(&eng(), &s, &[], date!(2026 - 01 - 05)).unwrap();
        let resumed = plan(&eng(), &s, &old, date!(2026 - 10 - 29)).unwrap();
        assert!(resumed[0].contains(date!(2026 - 10 - 29)));
        assert_eq!(resumed.len(), 4);
    }

    #[test]
    fn settings_ranges_are_checked() {
        let s = CycleSettings::standard();
        assert!(s.check().is_ok());
        assert!(
            CycleSettings {
                duration_weeks: 0,
                ..s
            }
            .check()
            .is_err()
        );
        assert!(
            CycleSettings {
                cooldown_weeks: 5,
                ..s
            }
            .check()
            .is_err()
        );
        assert!(CycleSettings { upcoming: 0, ..s }.check().is_err());
    }

    #[test]
    fn rollover_rules_follow_the_category() {
        assert_eq!(rollover_action(Category::Started), Rollover::MoveToNext);
        assert_eq!(rollover_action(Category::Unstarted), Rollover::MoveToNext);
        assert_eq!(rollover_action(Category::Completed), Rollover::Keep);
        assert_eq!(rollover_action(Category::Backlog), Rollover::Remove);
    }

    #[test]
    fn velocity_averages_the_last_three_finished_cycles() {
        let s = CycleSettings::standard();
        let mut cycles = plan(&eng(), &s, &[], date!(2026 - 06 - 01)).unwrap();
        for (c, pts) in cycles.iter_mut().zip([10, 20, 30, 40]) {
            c.record(BurnPoint {
                date: c.ends_on,
                scope: pts,
                started: 0,
                completed: pts,
                scope_issues: 1,
                completed_issues: 1,
            });
        }
        assert_eq!(velocity(&cycles, date!(2026 - 06 - 01)), None);
        assert_eq!(velocity(&cycles, date!(2026 - 06 - 29)), Some(15.0));
        assert_eq!(velocity(&cycles, date!(2026 - 08 - 01)), Some(30.0));
    }

    #[test]
    fn cycle_files_round_trip() {
        let c = plan(
            &eng(),
            &CycleSettings::standard(),
            &[],
            date!(2026 - 10 - 29),
        )
        .unwrap()
        .remove(0);
        assert_eq!(c.path(), "teams/ENG/cycles/2026-C01.yaml");
        let back = Cycle::from_yaml(&c.path(), &c.to_yaml().unwrap()).unwrap();
        assert_eq!(back, c);
        let bad = c.to_yaml().unwrap().replace("2026-11-09", "2026-10-01");
        assert!(Cycle::from_yaml("x", &bad).is_err());
    }
}
