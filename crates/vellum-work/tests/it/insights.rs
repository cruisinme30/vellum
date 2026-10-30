// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Insights from lifecycle timestamps on the Northstar demo workspace.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "fixture helpers panic on bad test data"
)]

use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};
use vellum_work::filter::{Condition, Field, Op};
use vellum_work::insights::{self, Interval, Measure, Pivot, Query};
use vellum_work::{
    Ctx, IssueId, IssuePatch, NewIssue, StatusKey, Team, TeamId, TeamKey, UserRef, WorkGraph,
};

fn user(h: &str) -> UserRef {
    UserRef::parse(h).unwrap()
}

fn tk(s: &str) -> TeamKey {
    TeamKey::parse(s).unwrap()
}

fn at(now: OffsetDateTime) -> Ctx {
    Ctx {
        now,
        today: now.date(),
        actor: Some(user("mara")),
    }
}

fn id(n: u32) -> IssueId {
    format!("01JC000000000000000000{n:04}").parse().unwrap()
}

const MON: OffsetDateTime = datetime!(2026-11-02 09:00 UTC);

/// Issue `n` for `who`, created at MON + `created` hours, started after
/// `wait` more hours and done after `work` more hours.
fn worked(g: &mut WorkGraph, n: u32, who: &str, created: i64, wait: i64, work: i64) {
    let t0 = MON + Duration::hours(created);
    let mut draft = NewIssue::new(id(n), tk("ENG"), "Work item");
    draft.assignee = Some(user(who));
    let _ = g.create_issue(draft, &at(t0)).unwrap();
    let t1 = t0 + Duration::hours(wait);
    let _ = g
        .set_state(&id(n), StatusKey::parse("in-progress").unwrap(), &at(t1))
        .unwrap();
    let t2 = t1 + Duration::hours(work);
    let _ = g
        .set_state(&id(n), StatusKey::parse("done").unwrap(), &at(t2))
        .unwrap();
}

fn northstar() -> WorkGraph {
    let mut g = WorkGraph::new();
    let mut eng = Team::new(
        "01JC00000000000000000000E1".parse::<TeamId>().unwrap(),
        tk("ENG"),
        "Engineering",
    )
    .unwrap();
    eng.members = vec![user("dev"), user("priya")];
    g.insert_team(eng).unwrap();
    // Week 1: dev finishes two, priya one. Week 2: priya one.
    worked(&mut g, 1, "dev", 0, 2, 10);
    worked(&mut g, 2, "dev", 1, 4, 20);
    worked(&mut g, 3, "priya", 2, 24, 30);
    worked(&mut g, 4, "priya", 170, 1, 5);
    // Still open: counts as created, never as done.
    let _ = g
        .create_issue(NewIssue::new(id(5), tk("ENG"), "Open"), &at(MON))
        .unwrap();
    g
}

fn query(measure: Measure, pivot: Pivot) -> Query {
    Query {
        filter: vellum_work::Filter::default(),
        measure,
        pivot,
        from: date!(2026 - 11 - 02),
        to: date!(2026 - 11 - 16),
        interval: None,
    }
}

#[test]
fn cycle_and_lead_time_by_assignee() {
    let g = northstar();
    let ctx = at(MON + Duration::weeks(3));
    let cycle = insights::run(&g, &query(Measure::CycleTime, Pivot::Assignee), &ctx, None).unwrap();
    assert_eq!(cycle.overall.count, 4);
    assert_eq!(cycle.overall.p50_hours, Some(10.0));
    assert_eq!(cycle.overall.p95_hours, Some(30.0));
    let keys: Vec<Option<&str>> = cycle.buckets.iter().map(|b| b.key.as_deref()).collect();
    assert_eq!(keys, [Some("dev"), Some("priya")]);
    assert_eq!(cycle.buckets[0].overall.mean_hours, Some(15.0));

    let lead = insights::run(&g, &query(Measure::LeadTime, Pivot::None), &ctx, None).unwrap();
    // Lead time adds the wait before work started: 12, 24, 54 and 6 hours.
    assert_eq!(lead.overall.p50_hours, Some(12.0));
    assert_eq!(lead.overall.mean_hours, Some(24.0));
}

#[test]
fn throughput_per_week_has_every_week() {
    let g = northstar();
    let mut q = query(Measure::Throughput, Pivot::None);
    q.interval = Some(Interval::Week);
    let r = insights::run(&g, &q, &at(MON + Duration::weeks(3)), None).unwrap();
    let weekly: Vec<(String, u32)> = r.buckets[0]
        .series
        .iter()
        .map(|p| (p.start.to_string(), p.stats.count))
        .collect();
    assert_eq!(
        weekly,
        [("2026-11-02".to_owned(), 3), ("2026-11-09".to_owned(), 1)]
    );
    assert_eq!(r.overall.p50_hours, None, "counts have no percentiles");

    let created = insights::run(&g, &query(Measure::Created, Pivot::None), &at(MON), None).unwrap();
    assert_eq!(created.overall.count, 5);
}

#[test]
fn filters_and_ranges_narrow_the_sample() {
    let mut g = northstar();
    let patch = IssuePatch {
        labels: Some(vec!["gateway".into()]),
        ..IssuePatch::default()
    };
    let mut team = g.team(&tk("ENG")).unwrap().clone();
    team.labels = vec![vellum_work::label::Label {
        name: "gateway".into(),
        group: None,
        color: None,
        description: None,
    }];
    let _ = g.update_team(team).unwrap();
    let _ = g
        .update_issue(&id(3), patch, &at(MON + Duration::weeks(3)))
        .unwrap();

    let mut q = query(Measure::CycleTime, Pivot::Label);
    q.filter = Condition::new(Field::Assignee, Op::Is, &["@me"]).into();
    let me = user("priya");
    let r = insights::run(&g, &q, &at(MON + Duration::weeks(3)), Some(&me)).unwrap();
    assert_eq!(r.overall.count, 2);
    let keys: Vec<Option<&str>> = r.buckets.iter().map(|b| b.key.as_deref()).collect();
    assert_eq!(keys, [None, Some("gateway")]);

    // The second week only.
    let mut q = query(Measure::CycleTime, Pivot::None);
    q.from = date!(2026 - 11 - 09);
    let r = insights::run(&g, &q, &at(MON + Duration::weeks(3)), None).unwrap();
    assert_eq!(r.overall.count, 1);
    assert_eq!(r.overall.p50_hours, Some(5.0));
}
