// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Work graph: teams, issues, cycles and projects, and the commands
//! that change them.
//!
//! The graph is the in-memory model the service layer loads from files and
//! keeps in step with them. Each command validates its input, changes the
//! graph, and returns a [`Changes`] listing the files to write and the
//! events to publish. The graph never touches storage, so every rule here
//! is a plain unit test.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use time::{Date, OffsetDateTime};
use tracing::{info, instrument};

use crate::cycle::{self, Cycle, CycleKey, Rollover};
use crate::error::{Result, WorkError};
use crate::event::{FieldChange, IssueSnapshot, WorkEvent};
use crate::filter::{EvalContext, Lookup};
use crate::id::{IssueId, MilestoneId, ProjectId, UserRef};
use crate::issue::{Issue, IssueKey, IssueLink, Relation};
use crate::label::Label;
use crate::project::{self, Project};
use crate::scale::{Estimate, Priority};
use crate::sla::{self, SlaChange};
use crate::team::{Team, TeamKey};
use crate::triage::{self, TriageAction};
use crate::workflow::{Category, StatusKey};

/// When and by whom a command runs.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The current time.
    pub now: OffsetDateTime,
    /// Today in the team's time zone (cycles and relative dates).
    pub today: Date,
    /// The person acting, if any (automations act as nobody).
    pub actor: Option<UserRef>,
}

/// The input for creating an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    /// The new issue's ID, from the kernel's generator.
    pub id: IssueId,
    /// The team.
    pub team: TeamKey,
    /// The title.
    pub title: String,
    /// The body.
    pub body: String,
    /// A template to apply first, by name.
    pub template: Option<String>,
    /// The status; unset means triage (for intake) or the default status.
    pub state: Option<StatusKey>,
    /// The priority.
    pub priority: Option<Priority>,
    /// The estimate.
    pub estimate: Option<Estimate>,
    /// The assignee.
    pub assignee: Option<UserRef>,
    /// Labels.
    pub labels: Vec<String>,
    /// The project.
    pub project: Option<ProjectId>,
    /// The milestone.
    pub milestone: Option<MilestoneId>,
    /// The cycle.
    pub cycle: Option<CycleKey>,
    /// The parent issue.
    pub parent: Option<IssueId>,
    /// Relations.
    pub relations: Vec<Relation>,
    /// The due date.
    pub due: Option<Date>,
    /// Links, such as the doc block the issue was created from.
    pub links: Vec<IssueLink>,
    /// Whether it arrived through intake (form, Slack, email, API from a
    /// non-member); intake issues go to triage when the team uses it.
    pub intake: bool,
}

impl NewIssue {
    /// A draft with only the required fields.
    #[must_use]
    pub fn new(id: IssueId, team: TeamKey, title: &str) -> Self {
        Self {
            id,
            team,
            title: title.to_owned(),
            body: String::new(),
            template: None,
            state: None,
            priority: None,
            estimate: None,
            assignee: None,
            labels: Vec::new(),
            project: None,
            milestone: None,
            cycle: None,
            parent: None,
            relations: Vec::new(),
            due: None,
            links: Vec::new(),
            intake: false,
        }
    }
}

/// A partial update. `Some(None)` clears a field; `None` leaves it alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::option_option, reason = "set, clear and keep are three states")]
pub struct IssuePatch {
    /// A new title.
    pub title: Option<String>,
    /// A new body.
    pub body: Option<String>,
    /// A new status.
    pub state: Option<StatusKey>,
    /// A new priority.
    pub priority: Option<Priority>,
    /// Set or clear the estimate.
    pub estimate: Option<Option<Estimate>>,
    /// Set or clear the assignee.
    pub assignee: Option<Option<UserRef>>,
    /// Replace the labels.
    pub labels: Option<Vec<String>>,
    /// Set or clear the project.
    pub project: Option<Option<ProjectId>>,
    /// Set or clear the milestone.
    pub milestone: Option<Option<MilestoneId>>,
    /// Set or clear the cycle.
    pub cycle: Option<Option<CycleKey>>,
    /// Set or clear the parent.
    pub parent: Option<Option<IssueId>>,
    /// Set or clear the due date.
    pub due: Option<Option<Date>>,
    /// Relations to add.
    pub add_relations: Vec<Relation>,
    /// Relations to remove.
    pub remove_relations: Vec<Relation>,
    /// People to subscribe.
    pub subscribe: Vec<UserRef>,
    /// People to unsubscribe.
    pub unsubscribe: Vec<UserRef>,
    /// Links to add (deduplicated by URL).
    pub add_links: Vec<IssueLink>,
}

/// What a command changed: the files to write and the events to publish.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Issues whose files must be written.
    pub issues: BTreeSet<IssueId>,
    /// Issue files to delete because the issue moved (old paths).
    pub removed_paths: BTreeSet<String>,
    /// Teams whose `team.yaml` must be written.
    pub teams: BTreeSet<TeamKey>,
    /// Cycles whose files must be written.
    pub cycles: BTreeSet<(TeamKey, CycleKey)>,
    /// Events, in order.
    pub events: Vec<WorkEvent>,
}

impl Changes {
    /// Adds what `other` changed, for a command made of several steps.
    pub fn merge(&mut self, other: Self) {
        self.issues.extend(other.issues);
        self.removed_paths.extend(other.removed_paths);
        self.teams.extend(other.teams);
        self.cycles.extend(other.cycles);
        self.events.extend(other.events);
    }
}

/// The Work model of one workspace.
#[derive(Debug, Clone, Default)]
pub struct WorkGraph {
    teams: BTreeMap<TeamKey, Team>,
    issues: BTreeMap<IssueId, Issue>,
    keys: HashMap<IssueKey, IssueId>,
    aliases: HashMap<IssueKey, IssueId>,
    cycles: BTreeMap<TeamKey, Vec<Cycle>>,
    projects: BTreeMap<ProjectId, Project>,
    labels: Vec<Label>,
}

impl Lookup for WorkGraph {
    fn category(&self, team: &TeamKey, state: &StatusKey) -> Option<Category> {
        self.teams.get(team)?.workflow.category_of(state)
    }

    fn is_open(&self, issue: &IssueId) -> bool {
        self.issues
            .get(issue)
            .and_then(|i| self.category(&i.team, &i.state))
            .is_some_and(|c| !c.is_closed())
    }

    fn current_cycle(&self, team: &TeamKey, today: Date) -> Option<CycleKey> {
        cycle::current(self.cycles.get(team)?, today).map(|c| c.key.clone())
    }
}

impl WorkGraph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ----- loading (the index is rebuilt from files) -----

    /// Adds a team loaded from `team.yaml` or newly created.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::DuplicateTeam`] if the key or ID is taken, or
    /// the [`Team::check`] error.
    pub fn insert_team(&mut self, team: Team) -> Result<()> {
        team.check()?;
        if self.teams.contains_key(&team.key)
            || self.teams.values().any(|t| t.id == team.id)
        {
            return Err(WorkError::DuplicateTeam(team.key));
        }
        let _ = self.teams.insert(team.key.clone(), team);
        Ok(())
    }

    /// Replaces a team's settings (workflow, cycles, rules, labels).
    ///
    /// Issues whose status no longer exists keep it in their files; views
    /// show them under "unknown" until someone moves them.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`] or the [`Team::check`] error.
    pub fn update_team(&mut self, mut team: Team) -> Result<Changes> {
        team.check()?;
        let old = self
            .teams
            .get(&team.key)
            .ok_or_else(|| WorkError::UnknownTeam(team.key.clone()))?;
        if team.id != old.id {
            return Err(WorkError::TeamConfig {
                team: team.key.clone(),
                reason: "the team ID cannot change".to_owned(),
            });
        }
        // A settings file read before an issue was filed must not hand its
        // number out again: numbers only grow.
        team.last_number = team.last_number.max(old.last_number);
        let mut changes = Changes::default();
        diff_labels(
            Some(&team.key),
            &old.labels,
            &team.labels,
            &mut changes.events,
        );
        let _ = changes.teams.insert(team.key.clone());
        let _ = self.teams.insert(team.key.clone(), team);
        Ok(changes)
    }

    /// Archives or restores a team (D303). An archived team keeps its
    /// issues, keys and history but takes no new issues, by creation, a
    /// triage rule or a move. Setting the state it already has changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`].
    pub fn set_team_archived(
        &mut self,
        key: &TeamKey,
        archived: bool,
    ) -> Result<Changes> {
        let team = self
            .teams
            .get_mut(key)
            .ok_or_else(|| WorkError::UnknownTeam(key.clone()))?;
        let mut changes = Changes::default();
        if team.archived != archived {
            team.archived = archived;
            let _ = changes.teams.insert(key.clone());
        }
        Ok(changes)
    }

    /// The team, refusing an archived one: it takes no new issues.
    fn open_team(&self, key: &TeamKey) -> Result<&Team> {
        let team = self.team_ref(key)?;
        if team.archived {
            return Err(WorkError::ArchivedTeam(key.clone()));
        }
        Ok(team)
    }

    /// Replaces the workspace-wide labels.
    pub fn set_workspace_labels(&mut self, labels: Vec<Label>) -> Changes {
        let mut changes = Changes::default();
        diff_labels(None, &self.labels, &labels, &mut changes.events);
        self.labels = labels;
        changes
    }

    /// Adds an issue loaded from its file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`], [`WorkError::DuplicateIssue`] or
    /// [`WorkError::DuplicateKey`] if the identifier or an alias is taken.
    pub fn load_issue(&mut self, issue: Issue) -> Result<()> {
        if !self.teams.contains_key(&issue.team) {
            return Err(WorkError::UnknownTeam(issue.team));
        }
        if self.issues.contains_key(&issue.id) {
            return Err(WorkError::DuplicateIssue(issue.id));
        }
        let key = issue.key();
        for k in std::iter::once(&key).chain(&issue.aliases) {
            if self.keys.contains_key(k) || self.aliases.contains_key(k) {
                return Err(WorkError::DuplicateKey(k.to_string()));
            }
        }
        let _ = self.keys.insert(key, issue.id);
        for a in &issue.aliases {
            let _ = self.aliases.insert(a.clone(), issue.id);
        }
        let _ = self.issues.insert(issue.id, issue);
        Ok(())
    }

    /// Removes an issue whose file was deleted. Its numbers stay used.
    pub fn unload_issue(&mut self, id: &IssueId) -> Option<Issue> {
        let issue = self.issues.remove(id)?;
        let _ = self.keys.remove(&issue.key());
        for a in &issue.aliases {
            let _ = self.aliases.remove(a);
        }
        if let Some(team) = self.teams.get_mut(&issue.team) {
            team.last_number = team.last_number.max(issue.number);
        }
        Some(issue)
    }

    /// Adds a cycle loaded from its file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`] or [`WorkError::Invalid`] if the
    /// team already has a cycle with this key.
    pub fn load_cycle(&mut self, c: Cycle) -> Result<()> {
        if !self.teams.contains_key(&c.team) {
            return Err(WorkError::UnknownTeam(c.team));
        }
        let list = self.cycles.entry(c.team.clone()).or_default();
        if list.iter().any(|x| x.key == c.key) {
            return Err(WorkError::Invalid {
                what: "cycle key",
                value: c.key.to_string(),
                reason: "already exists in the team",
            });
        }
        list.push(c);
        list.sort_by_key(|x| x.starts_on);
        Ok(())
    }

    /// Adds or replaces a project. Returns the important changes as an
    /// event when the project already existed.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`] if a team does not exist.
    pub fn upsert_project(&mut self, p: Project) -> Result<Changes> {
        if let Some(t) = p.teams.iter().find(|t| !self.teams.contains_key(*t)) {
            return Err(WorkError::UnknownTeam(t.clone()));
        }
        let mut changes = Changes::default();
        let event = match self.projects.get(&p.id) {
            None => Some(WorkEvent::ProjectCreated { project: p.id }),
            Some(old) => {
                let list: Vec<String> = project::changes(old, &p)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                (!list.is_empty()).then_some(WorkEvent::ProjectChanged {
                    project: p.id,
                    changes: list,
                })
            }
        };
        changes.events.extend(event);
        let _ = self.projects.insert(p.id, p);
        Ok(changes)
    }

    // ----- queries -----

    /// A team by key.
    #[must_use]
    pub fn team(&self, key: &TeamKey) -> Option<&Team> {
        self.teams.get(key)
    }

    /// Every team.
    pub fn teams(&self) -> impl Iterator<Item = &Team> {
        self.teams.values()
    }

    /// An issue by ID.
    #[must_use]
    pub fn issue(&self, id: &IssueId) -> Option<&Issue> {
        self.issues.get(id)
    }

    /// Every issue.
    pub fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.issues.values()
    }

    /// A team's cycles, oldest first.
    #[must_use]
    pub fn cycles(&self, team: &TeamKey) -> &[Cycle] {
        self.cycles.get(team).map_or(&[], Vec::as_slice)
    }

    /// A project by ID.
    #[must_use]
    pub fn project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects.get(id)
    }

    /// Every project.
    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.values()
    }

    /// The workspace-wide labels.
    #[must_use]
    pub fn workspace_labels(&self) -> &[Label] {
        &self.labels
    }

    /// Resolves an identifier (current or alias) or an ID to an issue.
    #[must_use]
    pub fn resolve(&self, text: &str) -> Option<&Issue> {
        if let Ok(key) = IssueKey::parse(text) {
            let id = self.keys.get(&key).or_else(|| self.aliases.get(&key))?;
            return self.issues.get(id);
        }
        text.parse::<IssueId>()
            .ok()
            .and_then(|id| self.issues.get(&id))
    }

    /// The category of an issue's status.
    #[must_use]
    pub fn category_of(&self, issue: &Issue) -> Option<Category> {
        self.category(&issue.team, &issue.state)
    }

    /// The points an issue counts for, using its team's unestimated value.
    #[must_use]
    pub fn points(&self, issue: &Issue) -> u32 {
        let fallback = self
            .teams
            .get(&issue.team)
            .map_or(1, |t| t.estimates.unestimated_points);
        issue
            .estimate
            .map_or(fallback, super::scale::Estimate::points)
    }

    /// The next number a team will hand out. It is above every number the
    /// team has used, including numbers now only aliases, so a number is
    /// never reused even if `team.yaml` lost its high-water mark.
    #[must_use]
    pub fn next_number(&self, team: &TeamKey) -> u64 {
        let recorded = self.teams.get(team).map_or(0, |t| t.last_number);
        let seen = self
            .keys
            .keys()
            .chain(self.aliases.keys())
            .filter(|k| &k.team == team)
            .map(|k| k.number)
            .max()
            .unwrap_or(0);
        recorded.max(seen) + 1
    }

    /// The team's triage inbox at `now`: visible triage issues, newest
    /// first.
    #[must_use]
    pub fn inbox(&self, team: &TeamKey, now: OffsetDateTime) -> Vec<&Issue> {
        let mut out: Vec<&Issue> = self
            .issues
            .values()
            .filter(|i| &i.team == team)
            .filter(|i| self.category_of(i) == Some(Category::Triage))
            .filter(|i| triage::is_visible(i, now))
            .collect();
        out.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        out
    }

    /// A team's progress per project, computed from issues.
    #[must_use]
    pub fn project_progress(&self, id: &ProjectId) -> project::ProjectProgress {
        project::progress(
            id,
            self.issues.values().filter_map(|i| {
                self.category_of(i).map(|c| (i, c, self.points(i)))
            }),
        )
    }

    /// An evaluation context for filters over this graph.
    #[must_use]
    pub fn eval_context<'a>(
        &'a self,
        ctx: &Ctx,
        me: Option<&'a UserRef>,
    ) -> EvalContext<'a> {
        EvalContext {
            today: ctx.today,
            now: ctx.now,
            me,
            lookup: self,
        }
    }

    fn snapshot(&self, issue: &Issue) -> IssueSnapshot {
        IssueSnapshot::of(
            issue,
            self.category_of(issue).unwrap_or(Category::Backlog),
        )
    }

    pub(crate) fn team_ref(&self, key: &TeamKey) -> Result<&Team> {
        self.teams
            .get(key)
            .ok_or_else(|| WorkError::UnknownTeam(key.clone()))
    }

    pub(crate) fn issue_ref(&self, id: &IssueId) -> Result<&Issue> {
        self.issues
            .get(id)
            .ok_or_else(|| WorkError::UnknownIssue(id.to_string()))
    }

    /// Replaces an issue whose key is unchanged.
    pub(crate) fn store_issue(&mut self, issue: Issue) {
        let _ = self.issues.insert(issue.id, issue);
    }

    // ----- validation helpers -----

    pub(crate) fn check_label(&self, team: &Team, label: &str) -> Result<()> {
        let known = team
            .labels
            .iter()
            .chain(&self.labels)
            .any(|l| l.reference().eq_ignore_ascii_case(label));
        if known {
            Ok(())
        } else {
            Err(WorkError::Invalid {
                what: "label",
                value: label.to_owned(),
                reason: "is not defined for the team or workspace",
            })
        }
    }

    fn check_fields(&self, issue: &Issue) -> Result<()> {
        let team = self.team_ref(&issue.team)?;
        if issue.title.trim().is_empty() {
            return Err(WorkError::Invalid {
                what: "title",
                value: issue.title.clone(),
                reason: "must not be empty",
            });
        }
        if team.workflow.get(&issue.state).is_none() {
            return Err(WorkError::UnknownStatus {
                team: team.key.clone(),
                status: issue.state.to_string(),
            });
        }
        if let Some(e) = issue.estimate {
            team.estimates.scale.check(e, team.estimates.allow_zero)?;
        }
        for l in &issue.labels {
            self.check_label(team, l)?;
        }
        if let Some(c) = &issue.cycle
            && !self.cycles(&issue.team).iter().any(|x| &x.key == c)
        {
            return Err(WorkError::Invalid {
                what: "cycle",
                value: c.to_string(),
                reason: "is not a cycle of the issue's team",
            });
        }
        if let Some(p) = &issue.project {
            let project =
                self.projects.get(p).ok_or_else(|| WorkError::Invalid {
                    what: "project",
                    value: p.to_string(),
                    reason: "does not exist",
                })?;
            if let Some(m) = &issue.milestone
                && project.milestone(m).is_none()
            {
                return Err(WorkError::Invalid {
                    what: "milestone",
                    value: m.to_string(),
                    reason: "is not a milestone of the issue's project",
                });
            }
        } else if issue.milestone.is_some() {
            return Err(WorkError::Invalid {
                what: "milestone",
                value: issue
                    .milestone
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                reason: "needs a project",
            });
        }
        if let Some(parent) = &issue.parent {
            self.check_parent(&issue.id, parent)?;
        }
        for r in &issue.relations {
            if r.issue == issue.id {
                return Err(WorkError::InvalidRelation {
                    issue: issue.key().to_string(),
                    reason: "an issue cannot relate to itself",
                });
            }
            let _ = self.issue_ref(&r.issue)?;
        }
        Ok(())
    }

    fn check_parent(&self, child: &IssueId, parent: &IssueId) -> Result<()> {
        let mut at = Some(parent);
        let mut steps = 0_usize;
        while let Some(p) = at {
            if p == child {
                return Err(WorkError::InvalidRelation {
                    issue: child.to_string(),
                    reason: "the parent chain would loop",
                });
            }
            steps += 1;
            if steps > self.issues.len() {
                break;
            }
            at = self.issue_ref(p)?.parent.as_ref();
        }
        Ok(())
    }

    // ----- commands -----

    /// Creates an issue.
    ///
    /// Applies the template, places the issue in triage (intake, when the
    /// team uses triage) or its status, runs triage rules, numbers it, adds
    /// it to the current cycle if it starts started, starts its SLA clock,
    /// and records inverse relations on the related issues.
    ///
    /// # Errors
    ///
    /// Returns a validation error and changes nothing if any field is
    /// invalid.
    #[instrument(skip_all, fields(team = %draft.team, issue = %draft.id))]
    pub fn create_issue(
        &mut self,
        draft: NewIssue,
        ctx: &Ctx,
    ) -> Result<Changes> {
        if self.issues.contains_key(&draft.id) {
            return Err(WorkError::DuplicateIssue(draft.id));
        }
        let mut issue = self.draft_to_issue(draft, ctx)?;
        // Triage rules route the issue before it gets a number.
        let rule_name = self.route_by_rules(&mut issue, ctx)?;
        // Checked after routing: a rule may point at an archived team.
        let _ = self.open_team(&issue.team)?;
        let category = self.category_of(&issue).ok_or_else(|| {
            WorkError::UnknownStatus {
                team: issue.team.clone(),
                status: issue.state.to_string(),
            }
        })?;
        if category == Category::Started || category == Category::Completed {
            let _ = issue.started_at.get_or_insert(ctx.now);
        }
        if category == Category::Completed {
            issue.completed_at = Some(ctx.now);
        }
        if category.is_closed() && category != Category::Completed {
            issue.canceled_at = Some(ctx.now);
        }
        self.auto_add_to_cycle(&mut issue, category, ctx.today);
        issue.number = self.next_number(&issue.team);
        self.check_fields(&issue)?;

        let mut changes = Changes::default();
        let policy = self.team_ref(&issue.team)?.sla.clone();
        let sla_change = sla::evaluate(
            &mut issue,
            category,
            &policy,
            &self.eval_context(ctx, None),
        );

        let team = self
            .teams
            .get_mut(&issue.team)
            .ok_or_else(|| WorkError::UnknownTeam(issue.team.clone()))?;
        team.last_number = issue.number;
        let _ = changes.teams.insert(issue.team.clone());
        let id = issue.id;
        let relations = issue.relations.clone();
        let _ = self.keys.insert(issue.key(), id);
        let _ = self.issues.insert(id, issue);
        for r in &relations {
            self.add_inverse(&id, r, ctx.now, &mut changes);
        }
        let _ = changes.issues.insert(id);
        let issue = self.issue_ref(&id)?;
        let snap = self.snapshot(issue);
        changes.events.push(WorkEvent::IssueCreated {
            issue: snap.clone(),
            triage_rule: rule_name,
        });
        push_sla_event(&sla_change, issue, snap, &mut changes);
        info!(
            monotonic_counter.work_issues_created = 1_u64,
            team = %issue.team,
            "issue created"
        );
        Ok(changes)
    }

    fn draft_to_issue(&self, mut draft: NewIssue, ctx: &Ctx) -> Result<Issue> {
        let team = self.team_ref(&draft.team)?;
        if let Some(name) = draft.template.clone() {
            let t = team.templates.iter().find(|t| t.name == name).ok_or(
                WorkError::Invalid {
                    what: "template",
                    value: name,
                    reason: "is not a template of the team",
                },
            )?;
            t.apply(&mut draft);
        }
        let creator_is_member =
            ctx.actor.as_ref().is_some_and(|a| team.is_member(a));
        let to_triage = draft.state.is_none()
            && team.triage
            && (draft.intake || !creator_is_member);
        let state = match &draft.state {
            Some(s) => s.clone(),
            None if to_triage => {
                team.workflow.first_in(Category::Triage).map_or_else(
                    || team.workflow.default.clone(),
                    |s| s.key.clone(),
                )
            }
            None => team.workflow.default.clone(),
        };
        let provisional = IssueKey {
            team: draft.team.clone(),
            number: u64::MAX,
        };
        let mut issue =
            Issue::new(draft.id, provisional, &draft.title, state, ctx.now);
        issue.body = draft.body;
        issue.priority = draft.priority.unwrap_or_default();
        issue.estimate = draft.estimate;
        issue.assignee = draft.assignee;
        issue.creator.clone_from(&ctx.actor);
        issue.labels = draft.labels;
        issue.project = draft.project;
        issue.milestone = draft.milestone;
        issue.cycle = draft.cycle;
        issue.parent = draft.parent;
        issue.relations = draft.relations;
        issue.due = draft.due;
        issue.links = draft.links;
        for who in issue.creator.iter().chain(&issue.assignee) {
            if !issue.subscribers.contains(who) {
                issue.subscribers.push(who.clone());
            }
        }
        Ok(issue)
    }

    /// Runs the team's triage rules on an issue entering triage and
    /// returns the name of the rule that matched.
    fn route_by_rules(
        &self,
        issue: &mut Issue,
        ctx: &Ctx,
    ) -> Result<Option<String>> {
        if self.category_of(issue) == Some(Category::Triage) {
            let ectx = self.eval_context(ctx, None);
            if let Some(rule) = triage::first_match(
                &self.team_ref(&issue.team)?.triage_rules,
                issue,
                &ectx,
            ) {
                let name = rule.name.clone();
                let actions = rule.then.clone();
                triage::apply_rule(issue, &actions);
                if let Some(target) = actions.team.filter(|t| t != &issue.team)
                {
                    let other = self.team_ref(&target)?;
                    let inbox = other
                        .workflow
                        .first_in(Category::Triage)
                        .ok_or_else(|| WorkError::TeamConfig {
                            team: target.clone(),
                            reason: "a triage rule routes here but the team \
                                has no triage"
                                .to_owned(),
                        })?;
                    issue.state = inbox.key.clone();
                    issue.team = target;
                    issue.cycle = None;
                }
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    fn auto_add_to_cycle(
        &self,
        issue: &mut Issue,
        category: Category,
        today: Date,
    ) {
        if issue.cycle.is_some() {
            return;
        }
        let Some(settings) = self.teams.get(&issue.team).and_then(|t| t.cycles)
        else {
            return;
        };
        let wanted = match category {
            Category::Started => settings.auto_add_started,
            Category::Completed => settings.auto_add_completed,
            _ => false,
        };
        if wanted {
            issue.cycle = self.current_cycle(&issue.team, today);
        }
    }

    fn add_inverse(
        &mut self,
        from: &IssueId,
        r: &Relation,
        now: OffsetDateTime,
        changes: &mut Changes,
    ) {
        let Some(kind) = r.kind.inverse() else { return };
        let inverse = Relation { kind, issue: *from };
        if let Some(other) = self.issues.get_mut(&r.issue)
            && !other.relations.contains(&inverse)
        {
            other.relations.push(inverse);
            other.updated_at = now;
            let _ = changes.issues.insert(other.id);
        }
    }

    fn remove_inverse(
        &mut self,
        from: &IssueId,
        r: &Relation,
        now: OffsetDateTime,
        changes: &mut Changes,
    ) {
        let Some(kind) = r.kind.inverse() else { return };
        let inverse = Relation { kind, issue: *from };
        if let Some(other) = self.issues.get_mut(&r.issue) {
            let before = other.relations.len();
            other.relations.retain(|x| x != &inverse);
            if other.relations.len() != before {
                other.updated_at = now;
                let _ = changes.issues.insert(other.id);
            }
        }
    }

    /// Changes fields of an issue.
    ///
    /// A status change keeps the lifecycle timestamps true and may add the
    /// issue to the current cycle; every change re-evaluates the SLA.
    ///
    /// # Errors
    ///
    /// Returns a validation error and changes nothing if any field is
    /// invalid.
    #[instrument(skip_all, fields(issue = %id))]
    pub fn update_issue(
        &mut self,
        id: &IssueId,
        patch: IssuePatch,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let before = self.issue_ref(id)?.clone();
        let workflow = self.team_ref(&before.team)?.workflow.clone();
        let mut issue = before.clone();
        let state = apply_patch(&mut issue, patch);
        let state_change = match state.filter(|s| s != &issue.state) {
            Some(s) => {
                let (from_cat, to_cat) =
                    issue.set_state(s, &workflow, ctx.now)?;
                Some((before.state.clone(), from_cat, to_cat))
            }
            None => None,
        };
        let category = workflow.category_of(&issue.state).ok_or_else(|| {
            WorkError::UnknownStatus {
                team: issue.team.clone(),
                status: issue.state.to_string(),
            }
        })?;
        if state_change.is_some() {
            self.auto_add_to_cycle(&mut issue, category, ctx.today);
        }
        self.check_fields(&issue)?;
        let policy = self.team_ref(&issue.team)?.sla.clone();
        let sla_change = sla::evaluate(
            &mut issue,
            category,
            &policy,
            &self.eval_context(ctx, None),
        );

        let field_changes = diff_fields(&before, &issue);
        if field_changes.is_empty()
            && state_change.is_none()
            && sla_change == SlaChange::Unchanged
        {
            return Ok(Changes::default());
        }
        issue.updated_at = ctx.now;
        let mut changes = Changes::default();
        let removed: Vec<Relation> = before
            .relations
            .iter()
            .filter(|r| !issue.relations.contains(r))
            .cloned()
            .collect();
        let added: Vec<Relation> = issue
            .relations
            .iter()
            .filter(|r| !before.relations.contains(r))
            .cloned()
            .collect();
        let _ = self.issues.insert(*id, issue);
        for r in &removed {
            self.remove_inverse(id, r, ctx.now, &mut changes);
        }
        for r in &added {
            self.add_inverse(id, r, ctx.now, &mut changes);
        }
        let _ = changes.issues.insert(*id);
        let issue = self.issue_ref(id)?;
        let snap = self.snapshot(issue);
        if let Some((from, from_category, _)) = state_change {
            changes.events.push(WorkEvent::IssueStateChanged {
                issue: snap.clone(),
                from,
                from_category,
            });
        }
        if !field_changes.is_empty() {
            changes.events.push(WorkEvent::IssueUpdated {
                issue: snap.clone(),
                changes: field_changes,
            });
        }
        push_sla_event(&sla_change, issue, snap, &mut changes);
        Ok(changes)
    }

    /// Moves an issue to the status `state`, a shortcut for Git automation
    /// and the `S` key.
    ///
    /// # Errors
    ///
    /// As [`WorkGraph::update_issue`].
    pub fn set_state(
        &mut self,
        id: &IssueId,
        state: StatusKey,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let patch = IssuePatch {
            state: Some(state),
            ..IssuePatch::default()
        };
        self.update_issue(id, patch, ctx)
    }

    /// Moves an issue to another team.
    ///
    /// The issue gets the next number in the new team and keeps its old
    /// identifier as an alias. Its status maps to the same category in the
    /// new workflow; its cycle is cleared; labels and an estimate the new
    /// team does not have are dropped.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownTeam`] or [`WorkError::UnknownIssue`].
    #[instrument(skip_all, fields(issue = %id, to = %to))]
    pub fn move_issue(
        &mut self,
        id: &IssueId,
        to: &TeamKey,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let before = self.issue_ref(id)?.clone();
        if &before.team == to {
            return Ok(Changes::default());
        }
        let category = self.category_of(&before).unwrap_or(Category::Backlog);
        let target = self.open_team(to)?;
        let mut issue = before.clone();
        issue.state = target.workflow.map_from(&before.state, category);
        issue.team = to.clone();
        issue.number = self.next_number(to);
        issue.cycle = None;
        if let Some(e) = issue.estimate
            && target
                .estimates
                .scale
                .check(e, target.estimates.allow_zero)
                .is_err()
        {
            issue.estimate = None;
        }
        issue.labels.retain(|l| self.check_label(target, l).is_ok());
        issue.aliases.push(before.key());
        issue.updated_at = ctx.now;

        let mut changes = Changes::default();
        let _ = changes.removed_paths.insert(before.path());
        let _ = self.keys.remove(&before.key());
        let _ = self.aliases.insert(before.key(), *id);
        let _ = self.keys.insert(issue.key(), *id);
        if let Some(t) = self.teams.get_mut(to) {
            t.last_number = issue.number;
        }
        if let Some(t) = self.teams.get_mut(&before.team) {
            t.last_number = t.last_number.max(before.number);
        }
        let _ = changes.teams.insert(to.clone());
        let _ = changes.teams.insert(before.team.clone());
        let _ = self.issues.insert(*id, issue);
        let _ = changes.issues.insert(*id);
        let issue = self.issue_ref(id)?;
        changes.events.push(WorkEvent::IssueMoved {
            issue: self.snapshot(issue),
            from: before.key(),
        });
        Ok(changes)
    }

    /// Applies a triage decision.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::InvalidState`] if the issue is not in triage,
    /// and [`WorkError::UnknownIssue`] if a duplicate target is unknown.
    #[instrument(skip_all, fields(issue = %id))]
    pub fn triage(
        &mut self,
        id: &IssueId,
        action: &TriageAction,
        ctx: &Ctx,
    ) -> Result<Changes> {
        if let TriageAction::Duplicate { of } = action {
            let _ = self.issue_ref(of)?;
        }
        let mut issue = self.issue_ref(id)?.clone();
        let team = self.team_ref(&issue.team)?;
        let workflow = team.workflow.clone();
        let policy = team.sla.clone();
        let (_, category) =
            triage::decide(&mut issue, action, &workflow, ctx.now)?;
        self.auto_add_to_cycle(&mut issue, category, ctx.today);
        let sla_change = sla::evaluate(
            &mut issue,
            category,
            &policy,
            &self.eval_context(ctx, None),
        );
        let _ = self.issues.insert(*id, issue);
        let mut changes = Changes::default();
        let _ = changes.issues.insert(*id);
        let issue = self.issue_ref(id)?;
        let snap = self.snapshot(issue);
        let name = match action {
            TriageAction::Accept { .. } => "accept",
            TriageAction::Decline { .. } => "decline",
            TriageAction::Duplicate { .. } => "duplicate",
            TriageAction::Snooze { .. } => "snooze",
        };
        changes.events.push(WorkEvent::IssueTriaged {
            issue: snap.clone(),
            action: name.to_owned(),
        });
        push_sla_event(&sla_change, issue, snap, &mut changes);
        info!(
            monotonic_counter.work_triage_decisions = 1_u64,
            action = name,
            "triage decision"
        );
        Ok(changes)
    }

    /// Creates any missing current and upcoming cycles for every team with
    /// cycles on.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if dates leave the supported range.
    #[instrument(skip_all)]
    pub fn ensure_cycles(&mut self, today: Date) -> Result<Changes> {
        let mut changes = Changes::default();
        let planned: Vec<(TeamKey, Vec<Cycle>)> = self
            .teams
            .values()
            .filter_map(|t| t.cycles.map(|s| (t, s)))
            .map(|(t, s)| {
                cycle::plan(&t.key, &s, self.cycles(&t.key), today)
                    .map(|c| (t.key.clone(), c))
            })
            .collect::<Result<_>>()?;
        for (team, new) in planned {
            for c in new {
                let _ = changes.cycles.insert((team.clone(), c.key.clone()));
                changes.events.push(WorkEvent::CycleCreated {
                    team: team.clone(),
                    cycle: c.key.clone(),
                });
                self.load_cycle(c)?;
            }
        }
        Ok(changes)
    }

    /// Rolls unfinished issues out of every finished cycle (D16): unstarted
    /// and started issues move to the current (or next) cycle, backlog and
    /// triage issues leave the cycle, closed issues stay.
    #[instrument(skip_all)]
    pub fn rollover(&mut self, now: OffsetDateTime, today: Date) -> Changes {
        let mut changes = Changes::default();
        let teams: Vec<TeamKey> = self
            .teams
            .values()
            .filter(|t| t.cycles.is_some_and(|c| c.auto_rollover))
            .map(|t| t.key.clone())
            .collect();
        for team in teams {
            let cycles = self.cycles(&team);
            let target = cycle::current(cycles, today)
                .or_else(|| cycle::next(cycles, today))
                .map(|c| c.key.clone());
            let past: Vec<CycleKey> = cycles
                .iter()
                .filter(|c| c.is_past(today))
                .map(|c| c.key.clone())
                .collect();
            for from in past {
                let mut moved = Vec::new();
                let mut removed = Vec::new();
                let ids: Vec<IssueId> = self
                    .issues
                    .values()
                    .filter(|i| {
                        i.team == team && i.cycle.as_ref() == Some(&from)
                    })
                    .map(|i| i.id)
                    .collect();
                for id in ids {
                    let category =
                        self.issues.get(&id).and_then(|i| self.category_of(i));
                    let action =
                        category.map_or(Rollover::Keep, cycle::rollover_action);
                    let Some(issue) = self.issues.get_mut(&id) else {
                        continue;
                    };
                    match (action, &target) {
                        (Rollover::MoveToNext, Some(t)) => {
                            issue.cycle = Some(t.clone());
                            moved.push(id);
                        }
                        (Rollover::MoveToNext, None)
                        | (Rollover::Remove, _) => {
                            issue.cycle = None;
                            removed.push(id);
                        }
                        (Rollover::Keep, _) => continue,
                    }
                    issue.updated_at = now;
                    let _ = changes.issues.insert(id);
                }
                if !moved.is_empty() || !removed.is_empty() {
                    changes.events.push(WorkEvent::CycleRolledOver {
                        team: team.clone(),
                        from,
                        to: target.clone(),
                        moved,
                        removed,
                    });
                }
            }
        }
        changes
    }

    /// Records today's burn-up point on every team's current cycle.
    #[instrument(skip_all)]
    pub fn record_burn_up(&mut self, today: Date) -> Changes {
        let mut changes = Changes::default();
        let mut points = Vec::new();
        for (team, cycles) in &self.cycles {
            let Some(c) = cycle::current(cycles, today) else {
                continue;
            };
            let unestimated = self
                .teams
                .get(team)
                .map_or(1, |t| t.estimates.unestimated_points);
            let issues = self
                .issues
                .values()
                .filter(|i| &i.team == team && i.cycle.as_ref() == Some(&c.key))
                .filter_map(|i| self.category_of(i).map(|cat| (i, cat)));
            points.push((
                team.clone(),
                c.key.clone(),
                cycle::snapshot(today, issues, unestimated),
            ));
        }
        for (team, key, point) in points {
            if let Some(c) = self
                .cycles
                .get_mut(&team)
                .and_then(|l| l.iter_mut().find(|c| c.key == key))
            {
                c.record(point);
                let _ = changes.cycles.insert((team, key));
            }
        }
        changes
    }

    /// Velocity of a team's cycles on `today`.
    #[must_use]
    pub fn velocity(&self, team: &TeamKey, today: Date) -> Option<f64> {
        cycle::velocity(self.cycles(team), today)
    }

    /// Checks every running SLA clock and reports risk step-ups.
    #[instrument(skip_all)]
    pub fn sla_tick(&mut self, now: OffsetDateTime) -> Changes {
        let mut changes = Changes::default();
        let ids: Vec<IssueId> = self
            .issues
            .values()
            .filter(|i| i.sla.as_ref().is_some_and(sla::Sla::is_running))
            .map(|i| i.id)
            .collect();
        for id in ids {
            let Some(issue) = self.issues.get(&id) else {
                continue;
            };
            let lead = self.teams.get(&issue.team).and_then(|t| t.lead.clone());
            let Some(issue) = self.issues.get_mut(&id) else {
                continue;
            };
            if let Some(n) = sla::tick(issue, lead.as_ref(), now) {
                let _ = changes.issues.insert(id);
                if let Some(issue) = self.issues.get(&id) {
                    changes.events.push(WorkEvent::SlaRisk {
                        issue: self.snapshot(issue),
                        risk: n.risk,
                        recipients: n.recipients,
                    });
                }
            }
        }
        changes
    }

    /// Runs the daily cycle automation: create cycles, roll over, record
    /// burn-up. The jobs service calls this once a day per team time zone.
    ///
    /// # Errors
    ///
    /// As [`WorkGraph::ensure_cycles`].
    pub fn daily(
        &mut self,
        now: OffsetDateTime,
        today: Date,
    ) -> Result<Changes> {
        let mut changes = self.ensure_cycles(today)?;
        changes.merge(self.rollover(now, today));
        changes.merge(self.record_burn_up(today));
        Ok(changes)
    }

    /// Sets or clears how often an issue repeats.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown issue,
    /// and [`WorkError::Invalid`] when a repeat is set on an issue with no
    /// due date (it would never fire).
    pub fn set_repeat(
        &mut self,
        id: &IssueId,
        repeat: Option<crate::recur::Repeat>,
        now: OffsetDateTime,
    ) -> Result<Changes> {
        let issue = self.issue_ref(id)?;
        if repeat == issue.repeat {
            return Ok(Changes::default());
        }
        if let Some(r) = repeat.filter(|_| issue.due.is_none()) {
            return Err(WorkError::Invalid {
                what: "repeat",
                value: r.to_string(),
                reason: "a repeating issue needs a due date",
            });
        }
        let issue = self
            .issues
            .get_mut(id)
            .ok_or_else(|| WorkError::UnknownIssue(id.to_string()))?;
        issue.repeat = repeat;
        issue.updated_at = now;
        let mut changes = Changes::default();
        let _ = changes.issues.insert(*id);
        Ok(changes)
    }

    /// Files the next issue of every repeating issue whose due date is before
    /// today in its team's time zone, as `today_of` gives it (D224): run every
    /// few minutes, that is 00:01 team time the day after. The copy keeps the
    /// team, title, description, priority, estimate, assignee, labels, project
    /// and milestone, starts in the team's default status and takes over the
    /// repeat; the old issue keeps its history and stops repeating.
    ///
    /// # Errors
    ///
    /// As [`WorkGraph::create_issue`]; nothing changes for the issue that
    /// failed, and earlier ones stay filed.
    pub fn recur(
        &mut self,
        now: OffsetDateTime,
        today_of: impl Fn(&Team) -> Date,
        mut new_id: impl FnMut() -> IssueId,
    ) -> Result<Changes> {
        let due: Vec<(IssueId, crate::recur::Repeat, Date, Date)> = self
            .issues
            .values()
            .filter_map(|i| {
                let today = today_of(self.teams.get(&i.team)?);
                Some((i.id, i.repeat?, i.due?, today))
            })
            .filter(|(_, _, due, today)| due < today)
            .collect();
        let mut changes = Changes::default();
        for (id, repeat, due_on, today) in due {
            let ctx = Ctx {
                now,
                today,
                actor: None,
            };
            let ctx = &ctx;
            let old = self.issue_ref(&id)?.clone();
            let Some(team) = self.teams.get(&old.team) else {
                continue;
            };
            let mut draft =
                NewIssue::new(new_id(), old.team.clone(), &old.title);
            draft.body.clone_from(&old.body);
            draft.state = Some(team.workflow.default.clone());
            draft.priority = Some(old.priority);
            draft.estimate = old.estimate;
            draft.assignee.clone_from(&old.assignee);
            draft.labels.clone_from(&old.labels);
            draft.project = old.project;
            draft.milestone = old.milestone;
            draft.due = Some(repeat.next_on_or_after(due_on, ctx.today));
            let next = draft.id;
            changes.merge(self.create_issue(draft, ctx)?);
            if let Some(i) = self.issues.get_mut(&next) {
                i.repeat = Some(repeat);
                i.creator.clone_from(&old.creator);
            }
            if let Some(i) = self.issues.get_mut(&id) {
                i.repeat = None;
                i.updated_at = ctx.now;
            }
            let _ = changes.issues.insert(id);
        }
        Ok(changes)
    }
}

/// Applies every field of a patch except the status, which it returns.
fn apply_patch(issue: &mut Issue, patch: IssuePatch) -> Option<StatusKey> {
    if let Some(t) = patch.title {
        issue.title = t;
    }
    if let Some(b) = patch.body {
        issue.body = b;
    }
    if let Some(p) = patch.priority {
        issue.priority = p;
    }
    if let Some(e) = patch.estimate {
        issue.estimate = e;
    }
    if let Some(a) = patch.assignee {
        if let Some(who) = a.as_ref().filter(|w| !issue.subscribers.contains(w))
        {
            issue.subscribers.push(who.clone());
        }
        issue.assignee = a;
    }
    if let Some(l) = patch.labels {
        let mut labels = Vec::new();
        for x in &l {
            triage::add_label(&mut labels, x);
        }
        issue.labels = labels;
    }
    if let Some(p) = patch.project {
        if p != issue.project {
            issue.milestone = None;
        }
        issue.project = p;
    }
    if let Some(m) = patch.milestone {
        issue.milestone = m;
    }
    if let Some(c) = patch.cycle {
        issue.cycle = c;
    }
    if let Some(p) = patch.parent {
        issue.parent = p;
    }
    if let Some(d) = patch.due {
        issue.due = d;
    }
    issue
        .relations
        .retain(|r| !patch.remove_relations.contains(r));
    for r in &patch.add_relations {
        if !issue.relations.contains(r) {
            issue.relations.push(r.clone());
        }
    }
    for s in &patch.subscribe {
        if !issue.subscribers.contains(s) {
            issue.subscribers.push(s.clone());
        }
    }
    issue.subscribers.retain(|s| !patch.unsubscribe.contains(s));
    for l in patch.add_links {
        if !issue.links.iter().any(|x| x.url == l.url) {
            issue.links.push(l);
        }
    }
    patch.state
}

fn push_sla_event(
    change: &SlaChange,
    issue: &Issue,
    snap: IssueSnapshot,
    changes: &mut Changes,
) {
    match (change, &issue.sla) {
        (SlaChange::Started | SlaChange::Resumed, Some(s)) => {
            changes.events.push(WorkEvent::SlaStarted {
                issue: snap,
                kind: s.kind.clone(),
                breaches_at: s.breaches_at,
            });
        }
        (SlaChange::Stopped { met }, _) => {
            changes.events.push(WorkEvent::SlaStopped {
                issue: snap,
                met: *met,
            });
        }
        _ => {}
    }
}

fn diff_labels(
    team: Option<&TeamKey>,
    old: &[Label],
    new: &[Label],
    events: &mut Vec<WorkEvent>,
) {
    let mut push = |label: &Label, change: &str| {
        events.push(WorkEvent::LabelChanged {
            team: team.cloned(),
            label: label.reference(),
            change: change.to_owned(),
        });
    };
    for l in new {
        match old.iter().find(|o| o.same_name(l)) {
            None => push(l, "created"),
            Some(o) if o != l => push(l, "updated"),
            Some(_) => {}
        }
    }
    for o in old {
        if !new.iter().any(|l| l.same_name(o)) {
            push(o, "deleted");
        }
    }
}

fn show<T: ToString>(v: Option<&T>) -> Option<String> {
    v.map(ToString::to_string)
}

fn diff_fields(a: &Issue, b: &Issue) -> Vec<FieldChange> {
    let mut out = Vec::new();
    let mut push = |field: &str, from: Option<String>, to: Option<String>| {
        if from != to {
            out.push(FieldChange {
                field: field.to_owned(),
                from,
                to,
            });
        }
    };
    push("title", Some(a.title.clone()), Some(b.title.clone()));
    if a.body != b.body {
        out_body(&mut push);
    }
    push(
        "priority",
        Some(a.priority.name().to_owned()),
        Some(b.priority.name().to_owned()),
    );
    push(
        "estimate",
        show(a.estimate.as_ref()),
        show(b.estimate.as_ref()),
    );
    push(
        "assignee",
        show(a.assignee.as_ref()),
        show(b.assignee.as_ref()),
    );
    push(
        "labels",
        Some(a.labels.join(", ")),
        Some(b.labels.join(", ")),
    );
    push(
        "project",
        show(a.project.as_ref()),
        show(b.project.as_ref()),
    );
    push(
        "milestone",
        show(a.milestone.as_ref()),
        show(b.milestone.as_ref()),
    );
    push("cycle", show(a.cycle.as_ref()), show(b.cycle.as_ref()));
    push("parent", show(a.parent.as_ref()), show(b.parent.as_ref()));
    push("due", show(a.due.as_ref()), show(b.due.as_ref()));
    let rel = |i: &Issue| {
        let mut v: Vec<String> = i
            .relations
            .iter()
            .map(|r| format!("{:?}:{}", r.kind, r.issue))
            .collect();
        v.sort();
        v.join(", ")
    };
    push("relations", Some(rel(a)), Some(rel(b)));
    let subs = |i: &Issue| {
        i.subscribers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    push("subscribers", Some(subs(a)), Some(subs(b)));
    let links = |i: &Issue| {
        i.links
            .iter()
            .map(|l| l.url.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    push("links", Some(links(a)), Some(links(b)));
    out
}

fn out_body(push: &mut impl FnMut(&str, Option<String>, Option<String>)) {
    // Bodies can be long; the event says the body changed, not how.
    push("body", None, Some("changed".to_owned()));
}
