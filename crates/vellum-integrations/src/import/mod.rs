// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Importing issues from Linear, Jira and GitHub Issues.
//!
//! Each reader turns an export into [`ImportedIssue`]s. [`plan`] then maps
//! them onto a Vellum team: states by category (and by name when the team
//! has a state of the same name), people by email or name, and priorities.
//! The issue core creates the issues in `created_at` order, which keeps
//! numbers stable on re-import, and records the source ID as an alias so
//! old keys (`ABC-12`, `owner/repo#7`) keep resolving. Relations and
//! parents are linked in a second pass with [`resolve_links`], once every
//! issue has its Vellum key. Imports are idempotent: issues whose source
//! ID was imported before are skipped.

pub mod csv;
pub mod github;
pub mod jira;
pub mod linear;

use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::model::{IssueDraft, IssueKey, Priority, Source, StateCategory, WorkflowState};

/// How two issues relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// This issue blocks the other.
    Blocks,
    /// This issue is blocked by the other.
    BlockedBy,
    /// Related.
    Related,
    /// This issue duplicates the other.
    DuplicateOf,
}

/// An issue read from another tracker.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ImportedIssue {
    /// Its ID in the source (`ABC-12`, `PROJ-7`, `owner/repo#3`).
    pub external_id: String,
    /// The title.
    pub title: String,
    /// The description as Markdown.
    pub description: String,
    /// The state's name in the source.
    pub state_name: String,
    /// The category the source state maps to.
    pub category: Option<StateCategory>,
    /// The priority.
    pub priority: Option<Priority>,
    /// The estimate as text.
    pub estimate: Option<String>,
    /// Label names.
    pub labels: BTreeSet<String>,
    /// The assignee (email, login or name).
    pub assignee: Option<String>,
    /// The creator (email, login or name).
    pub creator: Option<String>,
    /// The project or epic name.
    pub project: Option<String>,
    /// When it was created.
    pub created_at: Option<Timestamp>,
    /// When it was completed.
    pub completed_at: Option<Timestamp>,
    /// When it was canceled.
    pub canceled_at: Option<Timestamp>,
    /// When work started.
    pub started_at: Option<Timestamp>,
    /// The due date.
    pub due: Option<Date>,
    /// The parent's external ID.
    pub parent: Option<String>,
    /// Relations to other external IDs.
    pub relations: Vec<(Relation, String)>,
}

/// Something the import could not carry over exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// The issue it concerns, if any.
    pub external_id: Option<String>,
    /// What happened.
    pub message: String,
}

/// The result of reading an export.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Export {
    /// The issues, in source order.
    pub issues: Vec<ImportedIssue>,
    /// Problems found while reading.
    pub warnings: Vec<Warning>,
}

/// Why an export could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The file is not valid CSV or JSON of the expected shape.
    #[error("unreadable export: {0}")]
    Unreadable(String),
    /// A required column or field is missing.
    #[error("missing column `{0}`")]
    MissingColumn(String),
}

/// A person in the Vellum organisation, for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// User ID.
    pub id: String,
    /// Email address.
    pub email: String,
    /// Display name.
    pub name: String,
    /// Other handles, such as a GitHub login.
    pub handles: Vec<String>,
}

/// One planned creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The draft to create.
    pub draft: IssueDraft,
    /// The source ID, recorded as an alias.
    pub external_id: String,
    /// Lifecycle timestamps the core sets on creation.
    pub created_at: Option<Timestamp>,
    /// When it was started.
    pub started_at: Option<Timestamp>,
    /// When it was completed or canceled.
    pub closed_at: Option<Timestamp>,
}

/// The import plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Issues to create, oldest first.
    pub create: Vec<Planned>,
    /// Problems to show before the admin confirms.
    pub warnings: Vec<Warning>,
}

fn match_member<'m>(who: &str, members: &'m [Member]) -> Option<&'m Member> {
    let who = who.trim();
    members.iter().find(|m| {
        m.email.eq_ignore_ascii_case(who)
            || m.name.eq_ignore_ascii_case(who)
            || m.handles.iter().any(|h| h.eq_ignore_ascii_case(who))
    })
}

fn pick_state<'s>(issue: &ImportedIssue, states: &'s [WorkflowState]) -> Option<&'s WorkflowState> {
    states
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(issue.state_name.trim()))
        .or_else(|| {
            let category = issue.category.unwrap_or(StateCategory::Backlog);
            states.iter().find(|s| s.category == category)
        })
        .or_else(|| states.iter().find(|s| s.category == StateCategory::Backlog))
}

/// Maps an export onto `team`. `already` lists external IDs imported
/// before; they are skipped so re-running an import is safe.
#[must_use]
pub fn plan(
    tracker: &str,
    export: &Export,
    team: &str,
    states: &[WorkflowState],
    members: &[Member],
    already: &BTreeSet<String>,
) -> Plan {
    let mut plan = Plan {
        create: Vec::new(),
        warnings: export.warnings.clone(),
    };
    let mut seen = BTreeSet::new();
    let mut issues: Vec<&ImportedIssue> = export.issues.iter().collect();
    issues.sort_by_key(|i| i.created_at);
    for issue in issues {
        let warn = |message: String| Warning {
            external_id: Some(issue.external_id.clone()),
            message,
        };
        if already.contains(&issue.external_id) || !seen.insert(issue.external_id.clone()) {
            continue;
        }
        if issue.title.trim().is_empty() {
            plan.warnings.push(warn("skipped: no title".to_owned()));
            continue;
        }
        let mut draft = IssueDraft::new(
            team,
            crate::model::one_line(&issue.title, 255),
            Source::Import {
                tracker: tracker.to_owned(),
                external_id: issue.external_id.clone(),
            },
        );
        draft.description.clone_from(&issue.description);
        draft.priority = issue.priority;
        draft.estimate.clone_from(&issue.estimate);
        draft.labels.clone_from(&issue.labels);
        draft.project.clone_from(&issue.project);
        draft.due = issue.due;
        match pick_state(issue, states) {
            Some(state) => draft.state = Some(state.id.clone()),
            None => plan.warnings.push(warn(
                "no matching state; the team default is used".to_owned(),
            )),
        }
        if let Some(who) = &issue.assignee {
            match match_member(who, members) {
                Some(member) => draft.assignee = Some(member.id.clone()),
                None => plan.warnings.push(warn(format!(
                    "assignee `{who}` is not a member; left unassigned"
                ))),
            }
        }
        draft.reporter = issue
            .creator
            .as_deref()
            .map(|who| match_member(who, members).map_or_else(|| who.to_owned(), |m| m.id.clone()));
        plan.create.push(Planned {
            draft,
            external_id: issue.external_id.clone(),
            created_at: issue.created_at,
            started_at: issue.started_at,
            closed_at: issue.completed_at.or(issue.canceled_at),
        });
    }
    plan
}

/// A parent or relation to set once every issue exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Set `child`'s parent.
    Parent {
        /// The sub-issue.
        child: IssueKey,
        /// Its parent.
        parent: IssueKey,
    },
    /// Add a relation.
    Relation {
        /// The issue.
        from: IssueKey,
        /// How it relates.
        relation: Relation,
        /// The other issue.
        to: IssueKey,
    },
}

/// Resolves parents and relations once `created` maps every external ID
/// (this import's and earlier ones') to its Vellum key. Links to issues
/// outside the import are reported, not guessed.
#[must_use]
pub fn resolve_links(
    export: &Export,
    created: &BTreeMap<String, IssueKey>,
) -> (Vec<Link>, Vec<Warning>) {
    let mut links = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();
    for issue in &export.issues {
        let Some(from) = created.get(&issue.external_id) else {
            continue;
        };
        let mut resolve = |other: &str, what: &str| {
            let found = created.get(other).cloned();
            if found.is_none() {
                warnings.push(Warning {
                    external_id: Some(issue.external_id.clone()),
                    message: format!("{what} {other} was not imported"),
                });
            }
            found
        };
        if let Some(parent) = issue.parent.as_deref().and_then(|p| resolve(p, "parent")) {
            links.push(Link::Parent {
                child: from.clone(),
                parent,
            });
        }
        for (relation, other) in &issue.relations {
            if let Some(to) = resolve(other, "related issue") {
                // Store each relation once, from the side that blocks.
                let (a, r, b) = match relation {
                    Relation::BlockedBy => (to, Relation::Blocks, from.clone()),
                    Relation::Related if to < *from => (to, Relation::Related, from.clone()),
                    _ => (from.clone(), *relation, to),
                };
                if seen.insert((a.clone(), r, b.clone())) {
                    links.push(Link::Relation {
                        from: a,
                        relation: r,
                        to: b,
                    });
                }
            }
        }
    }
    (links, warnings)
}

/// Parses the date-time formats trackers export: RFC 3339, `YYYY-MM-DD
/// HH:MM:SS` (UTC), Jira's `+0000` offsets, and plain dates.
#[must_use]
pub fn timestamp(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(ts) = text.parse::<Timestamp>() {
        return Some(ts);
    }
    // Jira: 2026-10-29T08:00:00.000+0000
    if let Ok(zoned) =
        jiff::fmt::strtime::parse("%Y-%m-%dT%H:%M:%S%.f%z", text).and_then(|t| t.to_timestamp())
    {
        return Some(zoned);
    }
    let civil = text.replace(' ', "T");
    if let Ok(dt) = civil
        .trim_end_matches(" UTC")
        .parse::<jiff::civil::DateTime>()
    {
        return dt
            .to_zoned(jiff::tz::TimeZone::UTC)
            .ok()
            .map(|z| z.timestamp());
    }
    text.parse::<Date>()
        .ok()
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .map(|z| z.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: &str, name: &str, category: StateCategory) -> WorkflowState {
        WorkflowState {
            id: id.to_owned(),
            name: name.to_owned(),
            category,
        }
    }

    fn issue(id: &str, created: i64) -> ImportedIssue {
        ImportedIssue {
            external_id: id.to_owned(),
            title: format!("Issue {id}"),
            created_at: Timestamp::from_second(created).ok(),
            ..ImportedIssue::default()
        }
    }

    #[test]
    fn plans_states_people_and_order() {
        let states = [
            state("b", "Backlog", StateCategory::Backlog),
            state("t", "Todo", StateCategory::Unstarted),
            state("r", "In Review", StateCategory::Started),
            state("d", "Done", StateCategory::Completed),
        ];
        let members = [Member {
            id: "u1".to_owned(),
            email: "ada@northstar.dev".to_owned(),
            name: "Ada Park".to_owned(),
            handles: vec!["adap".to_owned()],
        }];
        let mut a = issue("X-2", 20);
        a.state_name = "in review".to_owned();
        a.assignee = Some("ADA@northstar.dev".to_owned());
        let mut b = issue("X-1", 10);
        b.category = Some(StateCategory::Completed);
        b.assignee = Some("ghost".to_owned());
        b.creator = Some("adap".to_owned());
        let mut c = issue("X-3", 30);
        c.title = "  ".to_owned();
        let export = Export {
            issues: vec![a, b, c, issue("X-4", 40)],
            warnings: Vec::new(),
        };
        let already: BTreeSet<String> = std::iter::once("X-4".to_owned()).collect();
        let plan = plan("linear", &export, "ENG", &states, &members, &already);
        let ids: Vec<&str> = plan.create.iter().map(|p| p.external_id.as_str()).collect();
        assert_eq!(ids, ["X-1", "X-2"]);
        let first = plan.create.first().map(|p| &p.draft);
        assert_eq!(first.and_then(|d| d.state.as_deref()), Some("d"));
        assert_eq!(first.and_then(|d| d.reporter.as_deref()), Some("u1"));
        let second = plan.create.get(1).map(|p| &p.draft);
        assert_eq!(second.and_then(|d| d.state.as_deref()), Some("r"));
        assert_eq!(second.and_then(|d| d.assignee.as_deref()), Some("u1"));
        let messages: Vec<&str> = plan.warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "assignee `ghost` is not a member; left unassigned",
                "skipped: no title"
            ]
        );
    }

    #[test]
    fn links_resolve_after_creation() {
        let mut a = issue("A", 1);
        a.parent = Some("P".to_owned());
        a.relations = vec![
            (Relation::BlockedBy, "B".to_owned()),
            (Relation::Related, "Z".to_owned()),
        ];
        let mut b = issue("B", 2);
        b.relations = vec![(Relation::Blocks, "A".to_owned())];
        let export = Export {
            issues: vec![a, b],
            warnings: Vec::new(),
        };
        let key = |n: u64| IssueKey {
            team: "ENG".to_owned(),
            number: n,
        };
        let created: BTreeMap<String, IssueKey> = [
            ("A".to_owned(), key(1)),
            ("B".to_owned(), key(2)),
            ("P".to_owned(), key(3)),
        ]
        .into_iter()
        .collect();
        let (links, warnings) = resolve_links(&export, &created);
        assert_eq!(
            links,
            vec![
                Link::Parent {
                    child: key(1),
                    parent: key(3)
                },
                Link::Relation {
                    from: key(2),
                    relation: Relation::Blocks,
                    to: key(1)
                },
            ]
        );
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn timestamps() {
        let expected = Timestamp::from_second(1_793_260_800).ok();
        assert_eq!(timestamp("2026-10-29T08:00:00Z"), expected);
        assert_eq!(timestamp("2026-10-29T08:00:00.000+0000"), expected);
        assert_eq!(timestamp("2026-10-29 08:00:00"), expected);
        assert!(timestamp("2026-10-29").is_some());
        assert_eq!(timestamp(""), None);
        assert_eq!(timestamp("yesterday"), None);
    }
}
