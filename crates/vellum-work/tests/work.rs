// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end rules of the Work graph, using the Northstar demo workspace.

#![allow(clippy::unwrap_used, reason = "fixture helpers panic on bad test data")]

use time::macros::{date, datetime};
use time::{Date, OffsetDateTime};
use vellum_work::cycle::CycleSettings;
use vellum_work::filter::{Condition, Field, Op};
use vellum_work::label::Label;
use vellum_work::my_issues::{Focus, Tab, my_issues};
use vellum_work::triage::{RuleActions, TriageRule};
use vellum_work::view::{GroupBy, Layout, OrderBy, ShowClosed, View, Visibility, run};
use vellum_work::{
    Category, Ctx, Cycle, Issue, IssueId, IssueKey, IssuePatch, NewIssue, Priority, Relation,
    RelationKind, StatusKey, Team, TeamId, TeamKey, TriageAction, UserRef, ViewId, WorkEvent,
    WorkGraph,
};

fn user(h: &str) -> UserRef {
    UserRef::parse(h).unwrap()
}

fn key(s: &str) -> StatusKey {
    StatusKey::parse(s).unwrap()
}

fn tk(s: &str) -> TeamKey {
    TeamKey::parse(s).unwrap()
}

fn id(n: u32) -> IssueId {
    format!("01JC000000000000000000{n:04}").parse().unwrap()
}

fn ctx(now: OffsetDateTime, actor: Option<&str>) -> Ctx {
    Ctx {
        now,
        today: now.date(),
        actor: actor.map(user),
    }
}

fn label(name: &str, group: Option<&str>) -> Label {
    Label {
        name: name.into(),
        group: group.map(Into::into),
        color: None,
        description: None,
    }
}

/// ENG (members dev, priya; lead mara; cycles on) and OPS (triage on).
fn northstar() -> WorkGraph {
    let mut g = WorkGraph::new();
    let mut eng = Team::new(
        "01JC00000000000000000000E1".parse::<TeamId>().unwrap(),
        tk("ENG"),
        "Engineering",
    )
    .unwrap();
    eng.lead = Some(user("mara"));
    eng.members = vec![user("dev"), user("priya")];
    eng.cycles = Some(CycleSettings::standard());
    eng.labels = vec![
        label("gateway", None),
        label("bug", Some("type")),
        label("feature", Some("type")),
    ];
    eng.triage_rules = vec![TriageRule {
        name: "Ops pages".into(),
        when: Condition::new(Field::Title, Op::Contains, &["pager"]).into(),
        then: RuleActions {
            team: Some(tk("OPS")),
            priority: Some(Priority::High),
            ..RuleActions::default()
        },
    }];
    g.insert_team(eng).unwrap();
    let ops = Team::new(
        "01JC00000000000000000000E2".parse::<TeamId>().unwrap(),
        tk("OPS"),
        "Operations",
    )
    .unwrap();
    g.insert_team(ops).unwrap();
    g
}

fn create(
    graph: &mut WorkGraph,
    n: u32,
    title: &str,
    edit: impl FnOnce(&mut NewIssue),
    c: &Ctx,
) -> IssueId {
    let mut draft = NewIssue::new(id(n), tk("ENG"), title);
    edit(&mut draft);
    let _ = graph.create_issue(draft, c).unwrap();
    id(n)
}

#[test]
fn members_create_into_the_default_status_and_numbers_increase() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let a = create(&mut g, 1, "Replay traffic", |_| {}, &c);
    let b = create(&mut g, 2, "Speed up replay", |_| {}, &c);
    assert_eq!(g.issue(&a).unwrap().key().to_string(), "ENG-1");
    assert_eq!(g.issue(&b).unwrap().key().to_string(), "ENG-2");
    assert_eq!(g.issue(&a).unwrap().state, key("backlog"));
    assert_eq!(g.issue(&a).unwrap().subscribers, [user("dev")]);
    assert_eq!(g.team(&tk("ENG")).unwrap().last_number, 2);
}

#[test]
fn intake_goes_to_triage_and_rules_route_it() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("outsider"));
    let changes = g
        .create_issue(NewIssue::new(id(1), tk("ENG"), "Login is slow"), &c)
        .unwrap();
    assert_eq!(g.issue(&id(1)).unwrap().state, key("triage"));
    assert!(matches!(
        &changes.events[0],
        WorkEvent::IssueCreated {
            triage_rule: None,
            ..
        }
    ));

    let changes = g
        .create_issue(NewIssue::new(id(2), tk("ENG"), "pager fires nightly"), &c)
        .unwrap();
    let routed = g.issue(&id(2)).unwrap();
    assert_eq!(routed.key().to_string(), "OPS-1");
    assert_eq!(routed.priority, Priority::High);
    assert!(
        matches!(&changes.events[0], WorkEvent::IssueCreated { triage_rule: Some(r), .. } if r == "Ops pages")
    );
    // High priority starts a one-week SLA clock.
    assert_eq!(routed.sla.as_ref().unwrap().kind, "high-1w");
    assert!(
        changes
            .events
            .iter()
            .any(|e| matches!(e, WorkEvent::SlaStarted { .. }))
    );

    let now = datetime!(2026-10-29 10:00 UTC);
    assert_eq!(g.inbox(&tk("ENG"), now).len(), 1);
    let _ = g
        .triage(
            &id(1),
            &TriageAction::Snooze {
                until: datetime!(2026-10-30 10:00 UTC),
            },
            &ctx(now, Some("dev")),
        )
        .unwrap();
    assert!(g.inbox(&tk("ENG"), now).is_empty());
    let _ = g
        .triage(
            &id(1),
            &TriageAction::Accept {
                status: Some(key("todo")),
            },
            &ctx(now, Some("dev")),
        )
        .unwrap();
    let accepted = g.issue(&id(1)).unwrap();
    assert_eq!(accepted.state, key("todo"));
    assert_eq!(accepted.triaged_at, Some(now));
    assert!(
        g.triage(&id(1), &TriageAction::Decline { reason: None }, &c)
            .is_err()
    );
}

#[test]
fn moving_keeps_the_old_identifier_as_an_alias() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let i = create(
        &mut g,
        1,
        "Rotate certificates",
        |d| {
            d.state = Some(key("in-review"));
            d.labels = vec!["gateway".into()];
            d.estimate = Some(vellum_work::Estimate::Points(3));
        },
        &c,
    );
    let changes = g.move_issue(&i, &tk("OPS"), &c).unwrap();
    let moved = g.issue(&i).unwrap();
    assert_eq!(moved.key().to_string(), "OPS-1");
    assert_eq!(moved.aliases, [IssueKey::parse("ENG-1").unwrap()]);
    assert_eq!(moved.state, key("in-review"), "same key, same category");
    assert!(moved.labels.is_empty(), "OPS has no gateway label");
    assert!(changes.removed_paths.contains("teams/ENG/issues/ENG-1.md"));
    assert_eq!(g.resolve("ENG-1").unwrap().id, i);
    assert_eq!(g.resolve("eng-1").unwrap().id, i);
    assert_eq!(g.resolve("OPS-1").unwrap().id, i);
    assert_eq!(g.resolve(&i.to_string()).unwrap().id, i);
    // ENG never reuses number 1.
    let next = create(&mut g, 2, "Next", |_| {}, &c);
    assert_eq!(g.issue(&next).unwrap().key().to_string(), "ENG-2");
}

#[test]
fn relations_are_two_way_and_validated() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let a = create(&mut g, 1, "Schema", |_| {}, &c);
    let b = create(
        &mut g,
        2,
        "API",
        |d| {
            d.relations = vec![Relation {
                kind: RelationKind::BlockedBy,
                issue: id(1),
            }];
        },
        &c,
    );
    assert!(g.issue(&a).unwrap().relations.contains(&Relation {
        kind: RelationKind::Blocks,
        issue: b
    }));

    let patch = IssuePatch {
        remove_relations: vec![Relation {
            kind: RelationKind::BlockedBy,
            issue: a,
        }],
        ..IssuePatch::default()
    };
    let _ = g.update_issue(&b, patch, &c).unwrap();
    assert!(g.issue(&a).unwrap().relations.is_empty());

    let self_rel = IssuePatch {
        add_relations: vec![Relation {
            kind: RelationKind::Related,
            issue: a,
        }],
        ..IssuePatch::default()
    };
    assert!(g.update_issue(&a, self_rel, &c).is_err());
    let loop_parent = IssuePatch {
        parent: Some(Some(b)),
        ..IssuePatch::default()
    };
    let _ = g.update_issue(&a, loop_parent, &c).unwrap();
    let back = IssuePatch {
        parent: Some(Some(a)),
        ..IssuePatch::default()
    };
    assert!(g.update_issue(&b, back, &c).is_err());
}

#[test]
fn commands_refuse_bad_fields_without_changing_anything() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let bad = [
        {
            let mut d = NewIssue::new(id(1), tk("ENG"), " ");
            d.state = Some(key("todo"));
            d
        },
        {
            let mut d = NewIssue::new(id(1), tk("ENG"), "x");
            d.state = Some(key("nope"));
            d
        },
        {
            let mut d = NewIssue::new(id(1), tk("ENG"), "x");
            d.labels = vec!["unknown".into()];
            d
        },
        {
            let mut d = NewIssue::new(id(1), tk("ENG"), "x");
            d.estimate = Some(vellum_work::Estimate::Points(4));
            d
        },
        {
            let mut d = NewIssue::new(id(1), tk("ENG"), "x");
            d.template = Some("none".into());
            d
        },
        {
            let mut d = NewIssue::new(id(1), tk("NOPE"), "x");
            d.state = None;
            d
        },
    ];
    for d in bad {
        assert!(g.create_issue(d, &c).is_err());
    }
    assert_eq!(g.issues().count(), 0);
    assert_eq!(g.next_number(&tk("ENG")), 1);
}

#[test]
fn state_changes_keep_lifecycle_and_sla_true() {
    let mut g = northstar();
    let t0 = datetime!(2026-10-29 09:00 UTC);
    let c = ctx(t0, Some("dev"));
    let i = create(
        &mut g,
        1,
        "Outage",
        |d| {
            d.priority = Some(Priority::Urgent);
            d.assignee = Some(user("dev"));
            d.state = Some(key("todo"));
        },
        &c,
    );
    assert_eq!(
        g.issue(&i).unwrap().sla.as_ref().unwrap().kind,
        "urgent-24h"
    );

    // Risk steps up: orange to the assignee, red to the assignee and lead.
    assert!(
        g.sla_tick(datetime!(2026-10-29 20:00 UTC))
            .events
            .is_empty()
    );
    let orange = g.sla_tick(datetime!(2026-10-30 04:00 UTC));
    assert!(
        matches!(&orange.events[0], WorkEvent::SlaRisk { recipients, .. } if recipients.len() == 1)
    );
    let red = g.sla_tick(datetime!(2026-10-30 10:00 UTC));
    assert!(
        matches!(&red.events[0], WorkEvent::SlaRisk { recipients, .. } if recipients.len() == 2)
    );

    let t1 = datetime!(2026-10-30 12:00 UTC);
    let changes = g.set_state(&i, key("done"), &ctx(t1, Some("dev"))).unwrap();
    let issue = g.issue(&i).unwrap();
    assert_eq!(issue.completed_at, Some(t1));
    assert_eq!(issue.started_at, Some(t1));
    assert!(
        changes
            .events
            .iter()
            .any(|e| matches!(e, WorkEvent::SlaStopped { met: false, .. }))
    );
    assert!(changes.events.iter().any(|e| matches!(
        e,
        WorkEvent::IssueStateChanged {
            from_category: Some(Category::Unstarted),
            ..
        }
    )));
    // A no-op patch changes nothing.
    assert_eq!(
        g.update_issue(&i, IssuePatch::default(), &ctx(t1, None))
            .unwrap(),
        vellum_work::Changes::default()
    );
}

fn day(d: Date) -> OffsetDateTime {
    d.with_hms(9, 0, 0).unwrap().assume_utc()
}

#[test]
fn cycles_run_themselves() {
    let mut g = northstar();
    let today = date!(2026 - 10 - 29);
    let created = g.daily(day(today), today).unwrap();
    assert_eq!(g.cycles(&tk("ENG")).len(), 4);
    assert_eq!(created.cycles.len(), 4);
    let current = g.cycles(&tk("ENG"))[0].key.clone();

    let c = ctx(day(today), Some("dev"));
    let started = create(
        &mut g,
        1,
        "Started",
        |d| d.state = Some(key("in-progress")),
        &c,
    );
    assert_eq!(
        g.issue(&started).unwrap().cycle.as_ref(),
        Some(&current),
        "auto-added"
    );
    let todo = create(
        &mut g,
        2,
        "Todo",
        |d| {
            d.state = Some(key("todo"));
            d.cycle = Some(current.clone());
            d.estimate = Some(vellum_work::Estimate::Points(5));
        },
        &c,
    );
    let done = create(&mut g, 3, "Done", |d| d.state = Some(key("done")), &c);
    let parked = create(&mut g, 4, "Parked", |d| d.cycle = Some(current.clone()), &c);

    let _ = g.record_burn_up(today);
    let point = *g.cycles(&tk("ENG"))[0].burn_up.last().unwrap();
    assert_eq!((point.scope, point.started, point.completed), (8, 1, 1));

    // Two weeks later the cycle has ended.
    let later = date!(2026 - 11 - 09);
    let changes = g.daily(day(later), later).unwrap();
    let next = g.cycles(&tk("ENG"))[1].key.clone();
    assert_eq!(g.issue(&started).unwrap().cycle.as_ref(), Some(&next));
    assert_eq!(g.issue(&todo).unwrap().cycle.as_ref(), Some(&next));
    assert_eq!(g.issue(&done).unwrap().cycle.as_ref(), Some(&current));
    assert_eq!(g.issue(&parked).unwrap().cycle, None);
    assert!(changes.events.iter().any(|e| matches!(e, WorkEvent::CycleRolledOver { moved, removed, .. } if moved.len() == 2 && removed.len() == 1)));
    assert_eq!(g.cycles(&tk("ENG")).len(), 5, "one more upcoming cycle");
    assert_eq!(g.velocity(&tk("ENG"), later), Some(1.0));
}

fn view(group_by: GroupBy, layout: Layout) -> View {
    View {
        id: "01JC00000000000000000000V1".parse::<ViewId>().unwrap(),
        name: "Board".into(),
        owner: user("dev"),
        team: Some(tk("ENG")),
        visibility: Visibility::Team,
        layout,
        filter: vellum_work::Filter::all(),
        group_by,
        order_by: OrderBy::Priority,
        show_closed: ShowClosed::All,
        show_sub_issues: true,
    }
}

#[test]
fn views_group_filter_and_order() {
    let mut g = northstar();
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let _ = create(
        &mut g,
        1,
        "Low",
        |d| {
            d.priority = Some(Priority::Low);
            d.state = Some(key("todo"));
            d.labels = vec!["gateway".into(), "type/bug".into()];
        },
        &c,
    );
    let _ = create(
        &mut g,
        2,
        "High",
        |d| {
            d.priority = Some(Priority::High);
            d.state = Some(key("todo"));
            d.assignee = Some(user("priya"));
        },
        &c,
    );
    let _ = create(
        &mut g,
        3,
        "Done",
        |d| {
            d.state = Some(key("done"));
            d.labels = vec!["gateway".into()];
        },
        &c,
    );

    let board = run(
        &view(GroupBy::Status, Layout::Board),
        &g,
        &c,
        Some(&user("dev")),
    );
    let columns: Vec<_> = board.iter().map(|grp| grp.key.clone().unwrap()).collect();
    assert_eq!(columns.first().unwrap(), "triage");
    assert!(
        columns.contains(&"in-review".to_owned()),
        "empty columns show on boards"
    );
    let todo = board
        .iter()
        .find(|grp| grp.key.as_deref() == Some("todo"))
        .unwrap();
    assert_eq!(todo.issues, [id(2), id(1)], "urgent first");

    let by_label = run(&view(GroupBy::Label, Layout::List), &g, &c, None);
    let gateway = by_label
        .iter()
        .find(|grp| grp.key.as_deref() == Some("gateway"))
        .unwrap();
    assert_eq!(gateway.issues.len(), 2);
    assert_eq!(by_label.last().unwrap().key, None, "no-label group last");

    let mut mine = view(GroupBy::Assignee, Layout::List);
    mine.filter = Condition::new(Field::Assignee, Op::Is, &["@me"]).into();
    let groups = run(&mine, &g, &c, Some(&user("priya")));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].issues, [id(2)]);

    let mut open_only = view(GroupBy::None, Layout::List);
    open_only.show_closed = ShowClosed::None;
    assert_eq!(run(&open_only, &g, &c, None)[0].issues.len(), 2);

    assert!(mine.check().is_ok());
    assert!(mine.visible_to(&user("x"), |t| t.as_str() == "ENG"));
    assert!(!mine.visible_to(&user("x"), |_| false));
    let yaml = serde_saphyr::to_string(&mine).unwrap();
    assert_eq!(serde_saphyr::from_str::<View>(&yaml).unwrap(), mine);
}

#[test]
fn my_issues_follow_focus_order() {
    let mut g = northstar();
    let today = date!(2026 - 10 - 29);
    let _ = g.daily(day(today), today).unwrap();
    let c = ctx(day(today), Some("dev"));
    let me = |d: &mut NewIssue| d.assignee = Some(user("dev"));
    let backlog = create(&mut g, 1, "Backlog", |d| me(d), &c);
    let done = create(
        &mut g,
        2,
        "Done",
        |d| {
            me(d);
            d.state = Some(key("done"));
        },
        &c,
    );
    let urgent = create(
        &mut g,
        3,
        "Urgent",
        |d| {
            me(d);
            d.priority = Some(Priority::Urgent);
            d.state = Some(key("todo"));
        },
        &c,
    );
    let sla = create(
        &mut g,
        4,
        "High",
        |d| {
            me(d);
            d.priority = Some(Priority::High);
            d.state = Some(key("todo"));
        },
        &c,
    );
    let active = create(
        &mut g,
        5,
        "Active",
        |d| {
            me(d);
            d.state = Some(key("todo"));
        },
        &c,
    );
    let in_cycle = create(
        &mut g,
        6,
        "Cycle",
        |d| {
            me(d);
            d.state = Some(key("in-progress"));
        },
        &c,
    );
    let blocked = create(
        &mut g,
        7,
        "Blocked",
        |d| {
            me(d);
            d.state = Some(key("todo"));
            d.relations = vec![Relation {
                kind: RelationKind::BlockedBy,
                issue: id(5),
            }];
        },
        &c,
    );
    let _other = create(&mut g, 8, "Not mine", |_| {}, &c);

    let list = my_issues(&g, &user("dev"), Tab::Assigned, &c, &[]);
    let order: Vec<(Focus, IssueId)> = list.iter().map(|(f, i)| (*f, i.id)).collect();
    assert_eq!(
        order,
        [
            (Focus::Urgent, urgent),
            (Focus::Sla, sla),
            (Focus::Blocked, blocked),
            (Focus::Cycle, in_cycle),
            (Focus::Active, active),
            (Focus::Backlog, backlog),
            (Focus::Done, done),
        ]
    );
    assert_eq!(my_issues(&g, &user("dev"), Tab::Created, &c, &[]).len(), 8);
    let activity = [
        (backlog, day(today)),
        (urgent, datetime!(2026-10-29 10:00 UTC)),
    ];
    let recent = my_issues(&g, &user("dev"), Tab::Activity, &c, &activity);
    assert_eq!(recent[0].1.id, urgent);
    assert!(my_issues(&g, &user("priya"), Tab::Subscribed, &c, &[]).is_empty());
}

#[test]
fn the_index_rebuilds_from_files() {
    let mut g = northstar();
    let today = date!(2026 - 10 - 29);
    let _ = g.daily(day(today), today).unwrap();
    let c = ctx(day(today), Some("dev"));
    let a = create(
        &mut g,
        1,
        "Parent",
        |d| d.state = Some(key("in-progress")),
        &c,
    );
    let _ = create(
        &mut g,
        2,
        "Child",
        |d| {
            d.parent = Some(id(1));
            d.priority = Some(Priority::Urgent);
            d.body = "- [ ] step one\n".into();
        },
        &c,
    );
    let _ = g.move_issue(&a, &tk("OPS"), &c).unwrap();

    // Write every file, then load them into a fresh graph.
    let teams: Vec<(String, String)> = g
        .teams()
        .map(|t| (t.path(), t.to_yaml().unwrap()))
        .collect();
    let cycles: Vec<(String, String)> = g
        .teams()
        .flat_map(|t| {
            g.cycles(&t.key)
                .iter()
                .map(|c| (c.path(), c.to_yaml().unwrap()))
        })
        .collect();
    let issues: Vec<(String, String)> = g
        .issues()
        .map(|i| (i.path(), i.to_markdown().unwrap()))
        .collect();

    let mut fresh = WorkGraph::new();
    for (p, t) in &teams {
        fresh.insert_team(Team::from_yaml(p, t).unwrap()).unwrap();
    }
    for (p, t) in &cycles {
        fresh.load_cycle(Cycle::from_yaml(p, t).unwrap()).unwrap();
    }
    for (p, t) in &issues {
        fresh
            .load_issue(Issue::from_markdown(p, t).unwrap())
            .unwrap();
    }
    for i in g.issues() {
        assert_eq!(fresh.issue(&i.id), Some(i));
    }
    assert_eq!(fresh.resolve("ENG-1").unwrap().id, a);
    assert_eq!(fresh.next_number(&tk("ENG")), 3);
    assert_eq!(fresh.next_number(&tk("OPS")), 2);
    // A second file claiming ENG-2 is refused.
    let mut dup = Issue::from_markdown(&issues[0].0, &issues[0].1).unwrap();
    dup.id = id(99);
    dup.number = 2;
    dup.team = tk("ENG");
    dup.aliases.clear();
    assert!(fresh.load_issue(dup).is_err());
}

#[test]
fn projects_report_progress_and_changes() {
    use vellum_work::project::{Cadence, Milestone, ProjectStatus};
    let mut g = northstar();
    let pid = "01JC00000000000000000000P1".parse().unwrap();
    let mut p = vellum_work::Project {
        id: pid,
        title: "Traffic replay".into(),
        lead: Some(user("dev")),
        members: vec![],
        teams: vec![tk("ENG")],
        status: ProjectStatus::InProgress,
        start_date: None,
        target_date: Some(date!(2026 - 12 - 01)),
        milestones: vec![Milestone {
            id: "01JC00000000000000000000M1".parse().unwrap(),
            name: "Alpha".into(),
            target_date: None,
        }],
        update_cadence: Cadence::Weekly,
        vellum: 1,
        body: String::new(),
    };
    let created = g.upsert_project(p.clone()).unwrap();
    assert!(matches!(
        created.events[0],
        WorkEvent::ProjectCreated { .. }
    ));
    let c = ctx(datetime!(2026-10-29 09:00 UTC), Some("dev"));
    let _ = create(
        &mut g,
        1,
        "A",
        |d| {
            d.project = Some(pid);
            d.state = Some(key("done"));
            d.estimate = Some(vellum_work::Estimate::Points(3));
        },
        &c,
    );
    let _ = create(
        &mut g,
        2,
        "B",
        |d| {
            d.project = Some(pid);
            d.milestone = Some(p.milestones[0].id);
        },
        &c,
    );
    let progress = g.project_progress(&pid);
    assert_eq!((progress.overall.scope, progress.overall.completed), (4, 3));
    assert_eq!(progress.overall.percent(), 75);
    assert_eq!(progress.milestones.len(), 1);

    p.target_date = Some(date!(2027 - 01 - 15));
    let changed = g.upsert_project(p.clone()).unwrap();
    assert!(
        matches!(&changed.events[0], WorkEvent::ProjectChanged { changes, .. } if changes[0].contains("2027-01-15"))
    );
    assert!(g.upsert_project(p).unwrap().events.is_empty());

    let wrong = {
        let mut d = NewIssue::new(id(3), tk("ENG"), "x");
        d.milestone = Some("01JC00000000000000000000M1".parse().unwrap());
        d
    };
    assert!(
        g.create_issue(wrong, &c).is_err(),
        "a milestone needs its project"
    );
}
