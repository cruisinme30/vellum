// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Planning over the gateway (R-WK-11, R-WK-12, D223): projects created
//! from fields, milestones, health updates, initiatives with rolled-up
//! health, and team templates in full.
//!
//! Updates are pages under `projects/<slug>/updates/`; initiatives are
//! `initiatives/<slug>.md`. Both are loaded at start-up next to the other
//! Work files and reloaded when another replica writes one.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};
use vellum_contracts::RequestContext;
use vellum_kernel::{Error, ErrorKind};
use vellum_work::project::{Cadence, Health, Milestone, ProjectStatus, rollup};
use vellum_work::{
    Changes, Initiative, InitiativeId, MilestoneId, Project, ProjectId,
    ProjectUpdate, Team, TeamKey, UpdateId, UserRef, WorkEvent,
};

use super::{State, WorkService, date, parse_id};
use crate::api::{
    ProjectOut, ProjectRef, TeamRef, invalid, name_of, parse_name, rfc3339,
};

/// The longest title or name this API accepts.
const MAX_TITLE: usize = 200;

/// The longest update or description body.
const MAX_BODY: usize = 64 * 1024;

/// Updates and initiatives, in memory.
#[derive(Debug, Clone, Default)]
pub struct Planning {
    /// Each project's updates, oldest first.
    updates: BTreeMap<ProjectId, Vec<ProjectUpdate>>,
    /// Initiatives with the path each was loaded from.
    initiatives: BTreeMap<InitiativeId, (String, Initiative)>,
}

impl Planning {
    /// Loads one update (`update`) or initiative page.
    pub(crate) fn load(
        &mut self,
        update: bool,
        path: &str,
        text: &str,
    ) -> Result<(), Error> {
        if update {
            self.add_update(ProjectUpdate::from_markdown(path, text)?);
        } else {
            let i = Initiative::from_markdown(path, text)?;
            let _ = self.initiatives.insert(i.id, (path.to_owned(), i));
        }
        Ok(())
    }

    fn add_update(&mut self, u: ProjectUpdate) {
        let list = self.updates.entry(u.project).or_default();
        list.retain(|x| x.id != u.id);
        let at = list.partition_point(|x| x.created_at <= u.created_at);
        list.insert(at, u);
    }

    /// A project's latest update.
    pub(crate) fn latest(&self, project: &ProjectId) -> Option<&ProjectUpdate> {
        self.updates.get(project).and_then(|l| l.last())
    }
}

/// Today in the team's time zone; UTC when the zone is unknown.
pub fn team_today(team: &Team, now: OffsetDateTime) -> Date {
    let utc = now.to_offset(time::UtcOffset::UTC).date();
    let Ok(tz) = jiff::tz::TimeZone::get(&team.timezone) else {
        return utc;
    };
    let Ok(at) = jiff::Timestamp::from_second(now.unix_timestamp()) else {
        return utc;
    };
    let local = at.to_zoned(tz).date();
    time::Month::try_from(u8::try_from(local.month()).unwrap_or(1))
        .ok()
        .and_then(|m| {
            Date::from_calendar_date(
                i32::from(local.year()),
                m,
                u8::try_from(local.day()).unwrap_or(1),
            )
            .ok()
        })
        .unwrap_or(utc)
}

/// A project as the API shows it.
pub fn project_out(st: &State, p: &Project, today: Date) -> ProjectOut {
    ProjectOut::of(
        p,
        &st.graph.project_progress(&p.id),
        st.planning.latest(&p.id),
        today,
    )
}

// ----- shapes -----

/// A new project.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateProject {
    /// Title.
    pub title: String,
    /// Team keys; at least one.
    pub teams: Vec<String>,
    /// Lead handle.
    pub lead: Option<String>,
    /// backlog (default), planned, in-progress, completed or canceled.
    pub status: Option<String>,
    /// Start date, `YYYY-MM-DD`.
    pub start_date: Option<String>,
    /// Target date, `YYYY-MM-DD`.
    pub target_date: Option<String>,
    /// weekly (default), biweekly, monthly or never.
    pub update_cadence: Option<String>,
    /// The spec, in Vellum Markdown.
    #[serde(default)]
    pub description: String,
}

/// A new milestone.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddMilestone {
    /// Project ID.
    pub project_id: String,
    /// Name.
    pub name: String,
    /// Target date, `YYYY-MM-DD`.
    pub target_date: Option<String>,
}

/// A health update to post.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PostUpdate {
    /// Project ID.
    pub project_id: String,
    /// on-track, at-risk or off-track.
    pub health: String,
    /// The notes, in Vellum Markdown.
    #[serde(default)]
    pub body: String,
}

/// A project update.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UpdateOut {
    /// Update ID.
    pub id: String,
    /// Author handle.
    pub author: String,
    /// When it was posted.
    pub created_at: String,
    /// on-track, at-risk or off-track.
    pub health: String,
    /// Important project changes since the update before.
    pub changes: Vec<String>,
    /// The notes, in Vellum Markdown.
    pub body: String,
}

impl UpdateOut {
    fn of(u: &ProjectUpdate) -> Self {
        Self {
            id: u.id.to_string(),
            author: u.author.to_string(),
            created_at: rfc3339(u.created_at),
            health: name_of(&u.health),
            changes: u.changes.clone(),
            body: u.body.clone(),
        }
    }
}

/// A project's updates.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UpdatesOut {
    /// Newest first.
    pub updates: Vec<UpdateOut>,
}

/// An initiative with its projects rolled up.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InitiativeOut {
    /// Initiative ID.
    pub id: String,
    /// Title.
    pub title: String,
    /// Owner handle.
    pub owner: Option<String>,
    /// Target date.
    pub target_date: Option<String>,
    /// The projects, in display order.
    pub projects: Vec<ProjectOut>,
    /// The worst latest health among active projects.
    pub health: Option<String>,
    /// Percent complete by points over every project.
    pub percent: u8,
    /// The initiative file, front matter and description.
    pub markdown: String,
}

/// Every initiative.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InitiativesOut {
    /// Initiatives by title.
    pub initiatives: Vec<InitiativeOut>,
}

/// A new initiative.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateInitiative {
    /// Title.
    pub title: String,
    /// Owner handle.
    pub owner: Option<String>,
    /// Target date, `YYYY-MM-DD`.
    pub target_date: Option<String>,
    /// Project IDs, in display order.
    #[serde(default)]
    pub projects: Vec<String>,
    /// The description, in Vellum Markdown.
    #[serde(default)]
    pub description: String,
}

/// Replace an initiative.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutInitiative {
    /// Initiative ID; must match the file.
    pub initiative_id: String,
    /// The whole initiative page.
    pub markdown: String,
}

/// A team's issue template.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TemplateOut {
    /// Name, unique within the team.
    pub name: String,
    /// Title to start from.
    pub title: Option<String>,
    /// Starting status slug.
    pub state: Option<String>,
    /// Priority 0–4.
    pub priority: Option<u8>,
    /// Estimate.
    pub estimate: Option<String>,
    /// Assignee handle.
    pub assignee: Option<String>,
    /// Labels added.
    pub labels: Vec<String>,
    /// Project ID.
    pub project: Option<String>,
    /// Repeat (`1w`).
    pub repeat: Option<String>,
    /// Body, in Vellum Markdown.
    pub body: String,
}

/// A team's templates.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TemplatesOut {
    /// In the team file's order.
    pub templates: Vec<TemplateOut>,
}

// ----- helpers -----

fn not_found(what: &str, id: &str) -> Error {
    Error::new(ErrorKind::NotFound, format!("unknown {what} {id}"))
}

fn title(text: &str, what: &str) -> Result<String, Error> {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > MAX_TITLE || t.contains(['\n', '\r'])
    {
        return Err(invalid(&format!(
            "{what}: one line of 1 to {MAX_TITLE} characters"
        )));
    }
    Ok(t.to_owned())
}

fn body(text: String) -> Result<String, Error> {
    if text.len() > MAX_BODY {
        return Err(invalid("the text is longer than 64 KiB"));
    }
    Ok(text)
}

fn user(text: Option<&str>) -> Result<Option<UserRef>, Error> {
    Ok(text
        .map(|t| t.trim().trim_start_matches('@'))
        .filter(|t| !t.is_empty())
        .map(UserRef::parse)
        .transpose()?)
}

fn day(text: Option<&str>) -> Result<Option<Date>, Error> {
    text.filter(|t| !t.is_empty()).map(date).transpose()
}

fn project_dir(st: &State, id: &ProjectId, p: &Project) -> String {
    let path = st
        .project_paths
        .get(id)
        .cloned()
        .unwrap_or_else(|| p.default_path());
    path.rsplit_once('/')
        .map_or_else(|| "projects".to_owned(), |(dir, _)| dir.to_owned())
}

fn initiative_out(
    st: &State,
    i: &Initiative,
    md: String,
    today: Date,
) -> InitiativeOut {
    let projects: Vec<&Project> = i
        .projects
        .iter()
        .filter_map(|id| st.graph.project(id))
        .collect();
    let health = rollup(
        projects
            .iter()
            .map(|p| (*p, st.planning.latest(&p.id).map(|u| u.health))),
    );
    let (mut scope, mut done) = (0_u64, 0_u64);
    for p in &projects {
        let progress = st.graph.project_progress(&p.id).overall;
        scope += u64::from(progress.scope);
        done += u64::from(progress.completed);
    }
    let percent = (done * 100)
        .checked_div(scope)
        .map_or(0, |p| u8::try_from(p.min(100)).unwrap_or(100));
    InitiativeOut {
        id: i.id.to_string(),
        title: i.title.clone(),
        owner: i.owner.as_ref().map(ToString::to_string),
        target_date: i.target_date.map(|d| d.to_string()),
        projects: projects.iter().map(|p| project_out(st, p, today)).collect(),
        health: health.map(|h| name_of(&h)),
        percent,
        markdown: md,
    }
}

// ----- operations -----

impl WorkService {
    pub(crate) async fn create_project(
        &self,
        rc: RequestContext,
        i: CreateProject,
    ) -> Result<ProjectOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let teams = i
            .teams
            .iter()
            .map(|t| Self::team_key(&st.graph, t))
            .collect::<Result<Vec<TeamKey>, _>>()?;
        if teams.is_empty() {
            return Err(invalid("a project needs at least one team"));
        }
        let project = Project {
            id: ProjectId::generate(),
            title: title(&i.title, "title")?,
            lead: user(i.lead.as_deref())?,
            members: Vec::new(),
            teams,
            status: i
                .status
                .as_deref()
                .map(|s| parse_name::<ProjectStatus>("status", s))
                .transpose()?
                .unwrap_or_default(),
            start_date: day(i.start_date.as_deref())?,
            target_date: day(i.target_date.as_deref())?,
            milestones: Vec::new(),
            update_cadence: i
                .update_cadence
                .as_deref()
                .map(|s| parse_name::<Cadence>("update_cadence", s))
                .transpose()?
                .unwrap_or_default(),
            vellum: 1,
            body: body(i.description)?,
        };
        self.save_project(&mut st, project, ctx.actor.as_ref(), ctx.today)
            .await
    }

    pub(crate) async fn add_milestone(
        &self,
        rc: RequestContext,
        i: AddMilestone,
    ) -> Result<ProjectOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        let mut project = st
            .graph
            .project(&id)
            .cloned()
            .ok_or_else(|| not_found("project", &i.project_id))?;
        project.milestones.push(Milestone {
            id: MilestoneId::generate(),
            name: title(&i.name, "name")?,
            target_date: day(i.target_date.as_deref())?,
        });
        self.save_project(&mut st, project, ctx.actor.as_ref(), ctx.today)
            .await
    }

    async fn save_project(
        &self,
        st: &mut State,
        project: Project,
        actor: Option<&UserRef>,
        today: Date,
    ) -> Result<ProjectOut, Error> {
        let id = project.id;
        let path = st
            .project_paths
            .get(&id)
            .cloned()
            .unwrap_or_else(|| project.default_path());
        let text = project.to_markdown()?;
        let backup = st.clone();
        let changes = st.graph.upsert_project(project)?;
        let _ = st.project_paths.insert(id, path.clone());
        self.commit(st, backup, changes, None, vec![(path, text)], actor)
            .await?;
        let p = st
            .graph
            .project(&id)
            .ok_or_else(|| not_found("project", &id.to_string()))?;
        Ok(project_out(st, p, today))
    }

    pub(crate) async fn list_updates(
        &self,
        _rc: RequestContext,
        i: ProjectRef,
    ) -> Result<UpdatesOut, Error> {
        let st = self.read_state().await;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        if st.graph.project(&id).is_none() {
            return Err(not_found("project", &i.project_id));
        }
        let updates = st
            .planning
            .updates
            .get(&id)
            .map(|l| l.iter().rev().map(UpdateOut::of).collect())
            .unwrap_or_default();
        Ok(UpdatesOut { updates })
    }

    pub(crate) async fn post_update(
        &self,
        rc: RequestContext,
        i: PostUpdate,
    ) -> Result<UpdateOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let author = ctx.actor.clone().ok_or_else(|| {
            Error::new(ErrorKind::PermissionDenied, "sign in to post")
        })?;
        let mut st = self.write_state().await?;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        let project = st
            .graph
            .project(&id)
            .ok_or_else(|| not_found("project", &i.project_id))?;
        let health = parse_name::<Health>("health", &i.health)?;
        let update = ProjectUpdate {
            id: UpdateId::generate(),
            project: id,
            author,
            created_at: ctx.now,
            health,
            changes: Vec::new(),
            vellum: 1,
            body: body(i.body)?,
        };
        let path = update.default_path(&project_dir(&st, &id, project));
        let text = update.to_markdown()?;
        let backup = st.clone();
        st.planning.add_update(update.clone());
        let changes = Changes {
            events: vec![WorkEvent::ProjectUpdatePosted {
                project: id,
                update: update.id,
                health,
            }],
            ..Changes::default()
        };
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            vec![(path, text)],
            ctx.actor.as_ref(),
        )
        .await?;
        Ok(UpdateOut::of(&update))
    }

    pub(crate) async fn list_initiatives(
        &self,
        _rc: RequestContext,
        _: crate::api::NoInput,
    ) -> Result<InitiativesOut, Error> {
        let st = self.read_state().await;
        let today = self.clock.now().date();
        let mut initiatives: Vec<InitiativeOut> = st
            .planning
            .initiatives
            .values()
            .map(|(_, i)| {
                initiative_out(
                    &st,
                    i,
                    i.to_markdown().unwrap_or_default(),
                    today,
                )
            })
            .collect();
        initiatives.sort_by_key(|a| a.title.to_lowercase());
        Ok(InitiativesOut { initiatives })
    }

    pub(crate) async fn create_initiative(
        &self,
        rc: RequestContext,
        i: CreateInitiative,
    ) -> Result<InitiativeOut, Error> {
        let initiative = Initiative {
            id: InitiativeId::generate(),
            title: title(&i.title, "title")?,
            owner: user(i.owner.as_deref())?,
            target_date: day(i.target_date.as_deref())?,
            projects: i
                .projects
                .iter()
                .map(|p| parse_id::<ProjectId>("projects", p))
                .collect::<Result<_, _>>()?,
            vellum: 1,
            body: body(i.description)?,
        };
        self.save_initiative(rc, initiative).await
    }

    pub(crate) async fn put_initiative(
        &self,
        rc: RequestContext,
        i: PutInitiative,
    ) -> Result<InitiativeOut, Error> {
        let id = parse_id::<InitiativeId>("initiative_id", &i.initiative_id)?;
        let initiative = Initiative::from_markdown(
            "initiatives/new.md",
            &body(i.markdown)?,
        )?;
        if initiative.id != id {
            return Err(invalid("the file's id must match initiative_id"));
        }
        let _ = title(&initiative.title, "title")?;
        self.save_initiative(rc, initiative).await
    }

    async fn save_initiative(
        &self,
        rc: RequestContext,
        initiative: Initiative,
    ) -> Result<InitiativeOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        if let Some(p) = initiative
            .projects
            .iter()
            .find(|p| st.graph.project(p).is_none())
        {
            return Err(not_found("project", &p.to_string()));
        }
        let path = st
            .planning
            .initiatives
            .get(&initiative.id)
            .map_or_else(|| initiative.default_path(), |(p, _)| p.clone());
        let text = initiative.to_markdown()?;
        let backup = st.clone();
        let _ = st
            .planning
            .initiatives
            .insert(initiative.id, (path.clone(), initiative.clone()));
        self.commit(
            &mut st,
            backup,
            Changes::default(),
            None,
            vec![(path, text.clone())],
            ctx.actor.as_ref(),
        )
        .await?;
        Ok(initiative_out(&st, &initiative, text, ctx.today))
    }

    pub(crate) async fn list_templates(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<TemplatesOut, Error> {
        let st = self.read_state().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let team = st
            .graph
            .team(&key)
            .ok_or_else(|| not_found("team", &i.team))?;
        Ok(TemplatesOut {
            templates: team
                .templates
                .iter()
                .map(|t| TemplateOut {
                    name: t.name.clone(),
                    title: t.title.clone(),
                    state: t.state.as_ref().map(ToString::to_string),
                    priority: t.priority.map(vellum_work::Priority::number),
                    estimate: t.estimate.map(|e| e.to_string()),
                    assignee: t.assignee.as_ref().map(ToString::to_string),
                    labels: t.labels.clone(),
                    project: t.project.map(|p| p.to_string()),
                    repeat: t.repeat.map(|r| r.to_string()),
                    body: t.body.clone(),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_one_short_line() {
        assert_eq!(title("  Launch  ", "title").unwrap(), "Launch");
        assert!(title("", "title").is_err());
        assert!(title("a\nb", "title").is_err());
        assert!(title(&"x".repeat(201), "title").is_err());
    }

    #[test]
    fn today_follows_the_team_zone() {
        let mut team = Team::new(
            "01JC00000000000000000000E1".parse().unwrap(),
            TeamKey::parse("ENG").unwrap(),
            "Engineering",
        )
        .unwrap();
        let now = time::macros::datetime!(2026-11-03 06:30 UTC);
        team.timezone = "America/Los_Angeles".to_owned();
        assert_eq!(team_today(&team, now), time::macros::date!(2026 - 11 - 02));
        team.timezone = "Asia/Tokyo".to_owned();
        assert_eq!(team_today(&team, now), time::macros::date!(2026 - 11 - 03));
        team.timezone = "Nowhere/Else".to_owned();
        assert_eq!(team_today(&team, now), time::macros::date!(2026 - 11 - 03));
    }

    #[test]
    fn handles_drop_the_at_sign() {
        assert_eq!(user(Some("@mara")).unwrap().unwrap().as_str(), "mara");
        assert!(user(Some("")).unwrap().is_none());
    }
}
