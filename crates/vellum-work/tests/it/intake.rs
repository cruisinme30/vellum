// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Intake forms and AI-suggested triage, end to end on the Northstar demo
//! workspace.

#![allow(clippy::unwrap_used, reason = "fixture helpers panic on bad test data")]

use std::collections::BTreeMap;

use time::OffsetDateTime;
use time::macros::datetime;
use vellum_work::intake::{FieldKind, FormField, IntakeForm, MapsTo};
use vellum_work::label::Label;
use vellum_work::suggest::TriageSuggestion;
use vellum_work::{
    Category, Ctx, IssueId, NewIssue, Priority, StatusKey, Team, TeamId, TeamKey, UserRef,
    WorkGraph,
};

fn user(h: &str) -> UserRef {
    UserRef::parse(h).unwrap()
}

fn tk(s: &str) -> TeamKey {
    TeamKey::parse(s).unwrap()
}

fn id(n: u32) -> IssueId {
    format!("01JC000000000000000000{n:04}").parse().unwrap()
}

fn ctx(actor: Option<&str>) -> Ctx {
    let now: OffsetDateTime = datetime!(2026-10-29 09:00 UTC);
    Ctx {
        now,
        today: now.date(),
        actor: actor.map(user),
    }
}

fn label(name: &str) -> Label {
    Label {
        name: name.into(),
        group: None,
        color: None,
        description: None,
    }
}

fn field(key: &str, kind: FieldKind, maps_to: MapsTo) -> FormField {
    FormField {
        key: key.into(),
        label: key.into(),
        kind,
        required: true,
        maps_to,
    }
}

/// ENG (members dev, priya) with a public bug form; OPS with triage off.
fn northstar() -> WorkGraph {
    let mut g = WorkGraph::new();
    let mut eng = Team::new(
        "01JC00000000000000000000E1".parse::<TeamId>().unwrap(),
        tk("ENG"),
        "Engineering",
    )
    .unwrap();
    eng.members = vec![user("dev"), user("priya")];
    eng.labels = vec![label("bug"), label("gateway")];
    eng.forms = vec![IntakeForm {
        slug: "bug".into(),
        title: "Report a bug".into(),
        description: String::new(),
        fields: vec![
            field("summary", FieldKind::Text, MapsTo::Title),
            field("steps", FieldKind::LongText, MapsTo::Description),
            field(
                "area",
                FieldKind::Select {
                    options: vec!["gateway".into(), "editor".into()],
                },
                MapsTo::Label,
            ),
        ],
        labels: vec!["bug".into()],
        public: true,
    }];
    g.insert_team(eng).unwrap();
    let mut ops = Team::new(
        "01JC00000000000000000000E2".parse::<TeamId>().unwrap(),
        tk("OPS"),
        "Operations",
    )
    .unwrap();
    ops.members = vec![user("sam")];
    ops.triage = false;
    g.insert_team(ops).unwrap();
    g
}

fn answers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn submit(g: &mut WorkGraph, n: u32, summary: &str) -> IssueId {
    let _ = g
        .submit_form(
            &tk("ENG"),
            "bug",
            id(n),
            &answers(&[
                ("summary", summary),
                ("steps", "Open the gateway\nWait"),
                ("area", "gateway"),
            ]),
            Some("visitor@example.com"),
            &ctx(None),
        )
        .unwrap();
    id(n)
}

fn suggestion() -> TriageSuggestion {
    TriageSuggestion {
        team: None,
        priority: Some(Priority::High),
        labels: Vec::new(),
        assignee: Some(user("priya")),
        duplicate_of: None,
        reason: "Gateway timeouts are Priya's area".into(),
        model: "assistant".into(),
        at: datetime!(2026-01-01 00:00 UTC),
    }
}

#[test]
fn a_public_form_files_a_triage_issue() {
    let mut g = northstar();
    let i = submit(&mut g, 1, "Gateway times out");
    let issue = g.issue(&i).unwrap();
    assert_eq!(issue.title, "Gateway times out");
    assert_eq!(issue.labels, ["bug", "gateway"]);
    assert_eq!(g.category_of(issue), Some(Category::Triage));
    assert!(issue.body.contains("by visitor@example.com"));

    // An unknown form, or a members-only form without a signed-in person,
    // files nothing.
    let mut bad = g.clone();
    assert!(
        bad.submit_form(&tk("ENG"), "nope", id(2), &answers(&[]), None, &ctx(None))
            .is_err()
    );
    assert!(g.issue(&id(2)).is_none());
}

#[test]
fn forms_survive_a_team_yaml_round_trip_and_bad_forms_are_refused() {
    let g = northstar();
    let team = g.team(&tk("ENG")).unwrap();
    let text = team.to_yaml().unwrap();
    let back = Team::from_yaml("teams/ENG/team.yaml", &text).unwrap();
    assert_eq!(back.forms, team.forms);

    let mut broken = team.clone();
    broken.forms.push(team.forms[0].clone());
    assert!(broken.check().is_err());
}

#[test]
fn a_suggestion_waits_for_a_person_then_applies() {
    let mut g = northstar();
    let i = submit(&mut g, 1, "Gateway times out");
    let brief = g.triage_brief(&i).unwrap();
    assert_eq!(brief.key, "ENG-1");
    assert_eq!(brief.teams.len(), 2);

    let _ = g.suggest_triage(&i, suggestion(), &ctx(None)).unwrap();
    // Nothing is applied yet; the suggestion is stored on the issue.
    let issue = g.issue(&i).unwrap();
    assert_eq!(issue.assignee, None);
    assert_eq!(g.category_of(issue), Some(Category::Triage));
    let stored = g.suggestion(&i).unwrap();
    assert_eq!(stored.at, datetime!(2026-10-29 09:00 UTC));

    let changes = g.accept_suggestion(&i, &ctx(Some("dev"))).unwrap();
    assert!(changes.issues.contains(&i));
    let issue = g.issue(&i).unwrap();
    assert_eq!(issue.assignee, Some(user("priya")));
    assert_eq!(issue.priority, Priority::High);
    assert_eq!(g.category_of(issue), Some(Category::Backlog));
    assert!(g.suggestion(&i).is_none());
}

#[test]
fn suggestions_are_checked_against_the_workspace() {
    let mut g = northstar();
    let i = submit(&mut g, 1, "Gateway times out");
    let bad = [
        TriageSuggestion {
            assignee: Some(user("sam")),
            ..suggestion()
        },
        TriageSuggestion {
            labels: vec!["unknown".into()],
            ..suggestion()
        },
        TriageSuggestion {
            duplicate_of: Some(i),
            ..suggestion()
        },
        TriageSuggestion {
            reason: "  ".into(),
            ..suggestion()
        },
        TriageSuggestion {
            team: Some(tk("NOPE")),
            ..suggestion()
        },
    ];
    for s in bad {
        assert!(
            g.suggest_triage(&i, s.clone(), &ctx(None)).is_err(),
            "{s:?}"
        );
    }
    assert!(g.suggestion(&i).is_none());
}

#[test]
fn a_duplicate_suggestion_closes_and_a_move_skips_triage() {
    let mut g = northstar();
    let _ = g
        .create_issue(
            NewIssue::new(id(9), tk("ENG"), "Gateway times out on upload"),
            &ctx(Some("dev")),
        )
        .unwrap();
    let a = submit(&mut g, 1, "Gateway times out");
    let brief = g.triage_brief(&a).unwrap();
    assert_eq!(brief.similar.first().map(|s| s.id), Some(id(9)));

    let dup = TriageSuggestion {
        duplicate_of: Some(id(9)),
        assignee: None,
        ..suggestion()
    };
    let _ = g.suggest_triage(&a, dup, &ctx(None)).unwrap();
    let _ = g.accept_suggestion(&a, &ctx(Some("dev"))).unwrap();
    assert_eq!(
        g.category_of(g.issue(&a).unwrap()),
        Some(Category::Duplicate)
    );

    let b = submit(&mut g, 2, "Disk full on db-2");
    let route = TriageSuggestion {
        team: Some(tk("OPS")),
        assignee: Some(user("sam")),
        ..suggestion()
    };
    let _ = g.suggest_triage(&b, route, &ctx(None)).unwrap();
    let _ = g.accept_suggestion(&b, &ctx(Some("dev"))).unwrap();
    let moved = g.issue(&b).unwrap();
    assert_eq!(moved.key().to_string(), "OPS-1");
    assert_eq!(moved.assignee, Some(user("sam")));
    assert_ne!(moved.state, StatusKey::parse("triage").unwrap());

    // Accepting with nothing waiting is refused.
    assert!(g.accept_suggestion(&b, &ctx(Some("dev"))).is_err());
}
