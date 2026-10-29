// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Projects, milestones, health updates and initiatives (R-WK-11).
//!
//! Progress is computed from issues; health is a human judgment written in
//! a project update. Important changes since the last update (target date
//! moved, status changed, milestones) are appended to the next update
//! automatically. Initiatives group projects and roll up the latest health.
//!
//! A project is also a page: `projects/<slug>/project.md` holds the fields
//! in front matter and the spec as its body; updates are pages under
//! `projects/<slug>/updates/`; initiatives are `initiatives/<slug>.md`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::{Date, Duration, OffsetDateTime};

use crate::error::WorkError;
use crate::id::{InitiativeId, MilestoneId, ProjectId, UpdateId, UserRef};
use crate::issue::{Issue, join_front_matter, slug, split_front_matter};
use crate::team::TeamKey;
use crate::workflow::Category;

/// A project's lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectStatus {
    /// Not planned yet.
    #[default]
    Backlog,
    /// Planned.
    Planned,
    /// Being worked on.
    InProgress,
    /// Done.
    Completed,
    /// Stopped.
    Canceled,
}

impl ProjectStatus {
    /// Whether the project is planned or in progress.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Planned | Self::InProgress)
    }
}

/// A project's health, set by a person in an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    /// On track.
    OnTrack,
    /// At risk.
    AtRisk,
    /// Off track.
    OffTrack,
}

/// How often a project's lead is reminded to post an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cadence {
    /// Every 7 days.
    #[default]
    Weekly,
    /// Every 14 days.
    Biweekly,
    /// Every 30 days.
    Monthly,
    /// No reminders.
    Never,
}

impl Cadence {
    const fn days(self) -> Option<i64> {
        match self {
            Self::Weekly => Some(7),
            Self::Biweekly => Some(14),
            Self::Monthly => Some(30),
            Self::Never => None,
        }
    }
}

/// A project milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Milestone {
    /// The milestone ID.
    pub id: MilestoneId,
    /// The name.
    pub name: String,
    /// The target date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_date: Option<Date>,
}

/// A project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The project ID.
    pub id: ProjectId,
    /// The name.
    pub title: String,
    /// The lead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<UserRef>,
    /// Members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<UserRef>,
    /// The teams working on it (one or more).
    pub teams: Vec<TeamKey>,
    /// The lifecycle status.
    #[serde(default)]
    pub status: ProjectStatus,
    /// The start date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<Date>,
    /// The target date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_date: Option<Date>,
    /// Milestones, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<Milestone>,
    /// How often updates are due.
    #[serde(default)]
    pub update_cadence: Cadence,
    /// The dialect version, always 1.
    #[serde(default = "one")]
    pub vellum: u8,
    /// The spec, in Vellum Markdown.
    #[serde(skip)]
    pub body: String,
}

const fn one() -> u8 {
    1
}

/// A project update: a health judgment with notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectUpdate {
    /// The update ID.
    pub id: UpdateId,
    /// The project.
    pub project: ProjectId,
    /// Who wrote it.
    pub author: UserRef,
    /// When it was posted.
    #[serde(with = "crate::timestamp")]
    pub created_at: OffsetDateTime,
    /// The health.
    pub health: Health,
    /// Changes since the previous update, appended automatically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// The dialect version, always 1.
    #[serde(default = "one")]
    pub vellum: u8,
    /// The notes, in Vellum Markdown.
    #[serde(skip)]
    pub body: String,
}

/// An initiative: a group of projects with rolled-up health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Initiative {
    /// The initiative ID.
    pub id: InitiativeId,
    /// The name.
    pub title: String,
    /// The owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<UserRef>,
    /// The target date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_date: Option<Date>,
    /// The projects, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectId>,
    /// The dialect version, always 1.
    #[serde(default = "one")]
    pub vellum: u8,
    /// The description, in Vellum Markdown.
    #[serde(skip)]
    pub body: String,
}

fn parse_page<T: serde::de::DeserializeOwned>(
    path: &str,
    text: &str,
) -> Result<(T, String), WorkError> {
    let err = |reason: String| WorkError::File {
        path: path.to_owned(),
        reason,
    };
    let (front, body) = split_front_matter(text).map_err(|r| err(r.to_owned()))?;
    let value = serde_yaml_ng::from_str(front).map_err(|e| err(e.to_string()))?;
    Ok((value, body.to_owned()))
}

fn write_page<T: Serialize>(value: &T, body: &str, path: &str) -> Result<String, WorkError> {
    let front = serde_yaml_ng::to_string(value).map_err(|e| WorkError::File {
        path: path.to_owned(),
        reason: e.to_string(),
    })?;
    Ok(join_front_matter(&front, body))
}

impl Project {
    /// The default path for a new project page.
    #[must_use]
    pub fn default_path(&self) -> String {
        format!(
            "projects/{}/project.md",
            slug_or_id(&self.title, &self.id.to_string())
        )
    }

    /// Parses a project page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the page is malformed or names no
    /// team.
    pub fn from_markdown(path: &str, text: &str) -> Result<Self, WorkError> {
        let (mut p, body): (Self, String) = parse_page(path, text)?;
        if p.teams.is_empty() {
            return Err(WorkError::File {
                path: path.to_owned(),
                reason: "a project needs at least one team".to_owned(),
            });
        }
        p.body = body;
        Ok(p)
    }

    /// Writes the project page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_markdown(&self) -> Result<String, WorkError> {
        write_page(self, &self.body, &self.default_path())
    }

    /// The milestone with this ID.
    #[must_use]
    pub fn milestone(&self, id: &MilestoneId) -> Option<&Milestone> {
        self.milestones.iter().find(|m| &m.id == id)
    }

    /// Whether an update is due on `today`, given the last update's time.
    #[must_use]
    pub fn update_due(&self, last_update: Option<OffsetDateTime>, today: Date) -> bool {
        if !self.status.is_active() {
            return false;
        }
        let Some(days) = self.update_cadence.days() else {
            return false;
        };
        last_update.is_none_or(|t| t.date() + Duration::days(days) <= today)
    }
}

impl ProjectUpdate {
    /// The default path, under the project's directory.
    #[must_use]
    pub fn default_path(&self, project_dir: &str) -> String {
        let id = self.id.to_string().to_ascii_lowercase();
        let short = id.get(id.len().saturating_sub(6)..).unwrap_or(&id);
        format!(
            "{project_dir}/updates/{}-{short}.md",
            self.created_at.date()
        )
    }

    /// Parses an update page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the page is malformed.
    pub fn from_markdown(path: &str, text: &str) -> Result<Self, WorkError> {
        let (mut u, body): (Self, String) = parse_page(path, text)?;
        u.body = body;
        Ok(u)
    }

    /// Writes the update page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_markdown(&self) -> Result<String, WorkError> {
        write_page(self, &self.body, &self.id.to_string())
    }
}

impl Initiative {
    /// The default path for a new initiative page.
    #[must_use]
    pub fn default_path(&self) -> String {
        format!(
            "initiatives/{}.md",
            slug_or_id(&self.title, &self.id.to_string())
        )
    }

    /// Parses an initiative page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the page is malformed.
    pub fn from_markdown(path: &str, text: &str) -> Result<Self, WorkError> {
        let (mut i, body): (Self, String) = parse_page(path, text)?;
        i.body = body;
        Ok(i)
    }

    /// Writes the initiative page.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_markdown(&self) -> Result<String, WorkError> {
        write_page(self, &self.body, &self.default_path())
    }
}

fn slug_or_id(title: &str, id: &str) -> String {
    let s = slug(title, 60);
    if s.is_empty() {
        id.to_ascii_lowercase()
    } else {
        s
    }
}

/// Computed progress over a set of issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Progress {
    /// Points in scope (canceled and duplicate issues excluded).
    pub scope: u32,
    /// Points in started issues.
    pub started: u32,
    /// Points in completed issues.
    pub completed: u32,
    /// Issues in scope.
    pub issues: u32,
    /// Completed issues.
    pub completed_issues: u32,
}

impl Progress {
    /// Adds one issue with its category and points.
    pub const fn add(&mut self, category: Category, points: u32) {
        if matches!(category, Category::Canceled | Category::Duplicate) {
            return;
        }
        self.scope = self.scope.saturating_add(points);
        self.issues = self.issues.saturating_add(1);
        match category {
            Category::Started => self.started = self.started.saturating_add(points),
            Category::Completed => {
                self.completed = self.completed.saturating_add(points);
                self.completed_issues = self.completed_issues.saturating_add(1);
            }
            _ => {}
        }
    }

    /// Completed points as a whole percentage of scope (0 when empty).
    #[must_use]
    pub fn percent(&self) -> u8 {
        if self.scope == 0 {
            return 0;
        }
        let pct = u64::from(self.completed) * 100 / u64::from(self.scope);
        u8::try_from(pct.min(100)).unwrap_or(100)
    }
}

/// A project's progress, overall and per milestone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectProgress {
    /// Over every issue in the project.
    pub overall: Progress,
    /// Per milestone, for issues that have one.
    pub milestones: BTreeMap<MilestoneId, Progress>,
}

/// Computes a project's progress from its issues, each given with its
/// category and points.
pub fn progress<'a>(
    project: &ProjectId,
    issues: impl IntoIterator<Item = (&'a Issue, Category, u32)>,
) -> ProjectProgress {
    let mut out = ProjectProgress::default();
    for (issue, category, points) in issues {
        if issue.project.as_ref() != Some(project) {
            continue;
        }
        out.overall.add(category, points);
        if let Some(m) = &issue.milestone {
            out.milestones.entry(*m).or_default().add(category, points);
        }
    }
    out
}

/// A change worth appending to the next project update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectChange {
    /// The target date moved.
    TargetDate(Option<Date>, Option<Date>),
    /// The start date moved.
    StartDate(Option<Date>, Option<Date>),
    /// The status changed.
    Status(ProjectStatus, ProjectStatus),
    /// The lead changed.
    Lead(Option<UserRef>, Option<UserRef>),
    /// A milestone was added.
    MilestoneAdded(String),
    /// A milestone was removed.
    MilestoneRemoved(String),
    /// A milestone's target date moved.
    MilestoneMoved(String, Option<Date>, Option<Date>),
}

fn date_or_none(d: Option<Date>) -> String {
    d.map_or_else(|| "none".to_owned(), |d| d.to_string())
}

impl fmt::Display for ProjectChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let day = |x: &Option<Date>| date_or_none(*x);
        let who = |x: &Option<UserRef>| {
            x.as_ref()
                .map_or_else(|| "nobody".to_owned(), |user| format!("@{user}"))
        };
        match self {
            Self::TargetDate(old, new) => {
                write!(f, "Target date moved from {} to {}", day(old), day(new))
            }
            Self::StartDate(old, new) => {
                write!(f, "Start date moved from {} to {}", day(old), day(new))
            }
            Self::Status(old, new) => write!(f, "Status changed from {old:?} to {new:?}"),
            Self::Lead(old, new) => write!(f, "Lead changed from {} to {}", who(old), who(new)),
            Self::MilestoneAdded(name) => write!(f, "Milestone {name} added"),
            Self::MilestoneRemoved(name) => write!(f, "Milestone {name} removed"),
            Self::MilestoneMoved(name, old, new) => {
                write!(
                    f,
                    "Milestone {name} moved from {} to {}",
                    day(old),
                    day(new)
                )
            }
        }
    }
}

/// The important changes between two versions of a project.
#[must_use]
pub fn changes(before: &Project, after: &Project) -> Vec<ProjectChange> {
    let mut out = Vec::new();
    if before.target_date != after.target_date {
        out.push(ProjectChange::TargetDate(
            before.target_date,
            after.target_date,
        ));
    }
    if before.start_date != after.start_date {
        out.push(ProjectChange::StartDate(
            before.start_date,
            after.start_date,
        ));
    }
    if before.status != after.status {
        out.push(ProjectChange::Status(before.status, after.status));
    }
    if before.lead != after.lead {
        out.push(ProjectChange::Lead(before.lead.clone(), after.lead.clone()));
    }
    for m in &after.milestones {
        match before.milestone(&m.id) {
            None => out.push(ProjectChange::MilestoneAdded(m.name.clone())),
            Some(old) if old.target_date != m.target_date => out.push(
                ProjectChange::MilestoneMoved(m.name.clone(), old.target_date, m.target_date),
            ),
            Some(_) => {}
        }
    }
    for m in &before.milestones {
        if after.milestone(&m.id).is_none() {
            out.push(ProjectChange::MilestoneRemoved(m.name.clone()));
        }
    }
    out
}

/// An initiative's health: the worst latest health among its active
/// projects, or `None` if none has an update.
pub fn rollup<'a>(
    latest: impl IntoIterator<Item = (&'a Project, Option<Health>)>,
) -> Option<Health> {
    latest
        .into_iter()
        .filter(|(p, _)| p.status.is_active())
        .filter_map(|(_, h)| h)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::IssueId;
    use crate::issue::IssueKey;
    use crate::workflow::StatusKey;
    use time::macros::{date, datetime};

    fn pid() -> ProjectId {
        "01JBZ3M8K2Q4R5S6T7V8W9X0P1".parse::<ProjectId>().unwrap()
    }

    fn mid(n: u8) -> MilestoneId {
        format!("01JBZ3M8K2Q4R5S6T7V8W9X0M{n}")
            .parse::<MilestoneId>()
            .unwrap()
    }

    pub fn project() -> Project {
        Project {
            id: pid(),
            title: "Traffic replay".into(),
            lead: Some(UserRef::parse("dev").unwrap()),
            members: vec![],
            teams: vec![TeamKey::parse("ENG").unwrap()],
            status: ProjectStatus::InProgress,
            start_date: Some(date!(2026 - 10 - 01)),
            target_date: Some(date!(2026 - 12 - 15)),
            milestones: vec![Milestone {
                id: mid(1),
                name: "Alpha".into(),
                target_date: Some(date!(2026 - 11 - 15)),
            }],
            update_cadence: Cadence::Weekly,
            vellum: 1,
            body: "## Spec\n".into(),
        }
    }

    #[test]
    fn pages_round_trip() {
        let p = project();
        assert_eq!(p.default_path(), "projects/traffic-replay/project.md");
        let back = Project::from_markdown("p", &p.to_markdown().unwrap()).unwrap();
        assert_eq!(back, p);
        let no_team = p
            .to_markdown()
            .unwrap()
            .replace("teams:\n- ENG\n", "teams: []\n");
        assert!(Project::from_markdown("p", &no_team).is_err());

        let u = ProjectUpdate {
            id: "01JBZ3M8K2Q4R5S6T7V8W9X0R1".parse::<UpdateId>().unwrap(),
            project: pid(),
            author: UserRef::parse("dev").unwrap(),
            created_at: datetime!(2026-10-29 10:00 UTC),
            health: Health::AtRisk,
            changes: vec!["Target date moved".into()],
            vellum: 1,
            body: "Load tests slipped.\n".into(),
        };
        assert_eq!(
            u.default_path("projects/traffic-replay"),
            "projects/traffic-replay/updates/2026-10-29-w9x0r1.md"
        );
        assert_eq!(
            ProjectUpdate::from_markdown("u", &u.to_markdown().unwrap()).unwrap(),
            u
        );

        let i = Initiative {
            id: "01JBZ3M8K2Q4R5S6T7V8W9X0N1"
                .parse::<InitiativeId>()
                .unwrap(),
            title: "Reliability 2027".into(),
            owner: None,
            target_date: None,
            projects: vec![pid()],
            vellum: 1,
            body: String::new(),
        };
        assert_eq!(i.default_path(), "initiatives/reliability-2027.md");
        assert_eq!(
            Initiative::from_markdown("i", &i.to_markdown().unwrap()).unwrap(),
            i
        );
    }

    #[test]
    fn progress_counts_points_and_milestones() {
        let mk = |n: u64, m: Option<MilestoneId>| {
            let mut i = Issue::new(
                format!("01JBZ3M8K2Q4R5S6T7V8W9X0A{n}")
                    .parse::<IssueId>()
                    .unwrap(),
                IssueKey::parse(&format!("ENG-{n}")).unwrap(),
                "x",
                StatusKey::parse("todo").unwrap(),
                datetime!(2026-10-01 00:00 UTC),
            );
            i.project = Some(pid());
            i.milestone = m;
            i
        };
        let done = mk(1, Some(mid(1)));
        let doing = mk(2, Some(mid(1)));
        let todo = mk(3, None);
        let dropped = mk(4, None);
        let mut other = mk(5, None);
        other.project = None;
        let p = progress(
            &pid(),
            [
                (&done, Category::Completed, 3),
                (&doing, Category::Started, 5),
                (&todo, Category::Unstarted, 2),
                (&dropped, Category::Canceled, 8),
                (&other, Category::Completed, 8),
            ],
        );
        assert_eq!(p.overall.scope, 10);
        assert_eq!(p.overall.completed, 3);
        assert_eq!(p.overall.started, 5);
        assert_eq!(p.overall.percent(), 30);
        assert_eq!(p.milestones[&mid(1)].scope, 8);
        assert_eq!(Progress::default().percent(), 0);
    }

    #[test]
    fn important_changes_are_listed() {
        let a = project();
        let mut b = project();
        b.target_date = Some(date!(2027 - 01 - 15));
        b.status = ProjectStatus::Completed;
        b.lead = None;
        b.milestones[0].target_date = None;
        b.milestones.push(Milestone {
            id: mid(2),
            name: "Beta".into(),
            target_date: None,
        });
        b.start_date = None;
        let text: Vec<String> = changes(&a, &b).iter().map(ToString::to_string).collect();
        assert_eq!(text[0], "Target date moved from 2026-12-15 to 2027-01-15");
        assert!(text.contains(&"Milestone Beta added".to_owned()));
        assert!(text.contains(&"Lead changed from @dev to nobody".to_owned()));
        assert!(text.iter().any(|t| t.starts_with("Milestone Alpha moved")));
        assert_eq!(text.len(), 6);
        assert_eq!(
            changes(&b, &a).last().unwrap(),
            &ProjectChange::MilestoneRemoved("Beta".into())
        );
    }

    #[test]
    fn health_rolls_up_to_the_worst_active_project() {
        let a = project();
        let mut done = project();
        done.status = ProjectStatus::Completed;
        assert_eq!(
            rollup([(&a, Some(Health::AtRisk)), (&done, Some(Health::OffTrack))]),
            Some(Health::AtRisk)
        );
        assert_eq!(rollup([(&a, None)]), None);
    }

    #[test]
    fn updates_fall_due_on_the_cadence() {
        let p = project();
        let today = date!(2026 - 10 - 29);
        assert!(p.update_due(None, today));
        assert!(!p.update_due(Some(datetime!(2026-10-25 09:00 UTC)), today));
        assert!(p.update_due(Some(datetime!(2026-10-22 09:00 UTC)), today));
        let mut never = project();
        never.update_cadence = Cadence::Never;
        assert!(!never.update_due(None, today));
        never.update_cadence = Cadence::Monthly;
        never.status = ProjectStatus::Backlog;
        assert!(!never.update_due(None, today));
    }
}
