// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`WorkService`]: the Work engine behind the gateway (D135–D137).

#![allow(
    clippy::significant_drop_tightening,
    reason = "each command holds the state lock until its files are written, so commands never interleave"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use tokio::sync::Mutex;
use tracing::{info, instrument, warn};
use vellum_contracts::{Operation, RequestContext, Scope};
use vellum_kernel::{CommentId, Error, ErrorKind, TeamId};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};
use vellum_work::comment::{AuthoredBy, CommentChanges, Comments, NewComment, Target, Thread};
use vellum_work::cycle::{Cycle, CycleKey};
use vellum_work::doclink;
use vellum_work::filter::Filter;
use vellum_work::my_issues::{Tab, my_issues};
use vellum_work::project::Project;
use vellum_work::view::{self, View, Visibility};
use vellum_work::{
    BlockRef, Changes, Ctx, Estimate, Issue, IssueId, IssuePatch, NewIssue, Priority, ProjectId,
    Relation, StatusKey, Team, TeamKey, TriageAction, UserRef, ViewId, WorkGraph,
};

use crate::api::{
    self, AddComment, AssignThread, AssignmentOut, AssignmentsOut, ChipOut, CommentRef,
    CommentResult, CreateIssue, CreateTeam, CreatedIssue, CycleOut, CyclesOut, EditComment,
    FocusedIssue, GroupOut, IssueGroups, IssueOut, IssueRef, ListComments, MoveIssue, MyIssues,
    MyIssuesOut, NoInput, ProjectOut, ProjectRef, PutProject, PutTeam, QueryIssues, React,
    ResolveThread, TeamOut, TeamRef, ThreadOut, ThreadsOut, TriageIssue, UpdateIssue, invalid,
    parse_name,
};
use crate::ports::{Clock, Directory, EventSink};
use crate::routes;

pub mod embed;
pub mod embeds;
pub mod insights;
pub mod live;
pub mod triage;

/// The most issues one list returns.
pub const MAX_LIST: usize = 1000;

/// The default number of issues in a list.
pub const DEFAULT_LIST: usize = 250;

/// Everything Work holds in memory, rebuilt from files at start-up.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub(crate) graph: WorkGraph,
    pub(crate) comments: Comments,
    /// Where each project page lives; titles can change, paths do not.
    project_paths: BTreeMap<ProjectId, String>,
}

/// The Work service: issue tracking and comments for one workspace.
///
/// It owns the Work files in the workspace's blob store (`teams/`,
/// `projects/`, `.comments/`). Every command runs under one lock: the
/// engine validates and applies it, the changed files are written, and only
/// then are events published. If a write fails, memory is put back as it
/// was and the caller gets `unavailable`.
pub struct WorkService {
    store: Arc<dyn BlobStore>,
    directory: Arc<dyn Directory>,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

impl std::fmt::Debug for WorkService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkService")
            .field("store", &self.store.backend())
            .finish_non_exhaustive()
    }
}

/// How many files a load read and skipped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// Files loaded.
    pub loaded: usize,
    /// Files that could not be read or parsed; each is logged.
    pub skipped: usize,
}

impl WorkService {
    /// Loads every Work file from `store` and returns the service.
    ///
    /// A file that cannot be parsed is logged and skipped, so one bad hand
    /// edit never takes Work down.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` if the store cannot be listed.
    #[instrument(skip_all, fields(backend = store.backend()))]
    pub async fn open(
        store: Arc<dyn BlobStore>,
        directory: Arc<dyn Directory>,
        events: Arc<dyn EventSink>,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, LoadReport), Error> {
        let mut state = State::default();
        let mut report = LoadReport::default();
        let mut paths = Vec::new();
        for dir in ["teams", "projects", ".comments"] {
            paths.extend(list(store.as_ref(), dir).await?);
        }
        // Teams first: issues and cycles need their team.
        paths.sort_by_key(|p| (!p.ends_with("/team.yaml"), p.clone()));
        for path in paths {
            match load_one(store.as_ref(), &mut state, &path).await {
                Ok(true) => report.loaded += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!(path, error = %e, "skipped a Work file");
                    report.skipped += 1;
                }
            }
        }
        info!(
            loaded = report.loaded,
            skipped = report.skipped,
            "Work loaded"
        );
        let service = Self {
            store,
            directory,
            events,
            clock,
            state: Mutex::new(state),
        };
        Ok((service, report))
    }

    /// Runs the daily automation: cycles created, rolled over and burn-up
    /// recorded, then SLA risk notices. The jobs service calls this once a
    /// day per team time zone.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` if the files cannot be written.
    #[instrument(skip_all)]
    pub async fn run_daily(&self) -> Result<(), Error> {
        let now = self.clock.now();
        let mut st = self.state.lock().await;
        let backup = st.clone();
        let mut changes = st.graph.daily(now, now.date())?;
        let sla = st.graph.sla_tick(now);
        changes.issues.extend(sla.issues);
        changes.events.extend(sla.events);
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await
    }

    // ----- helpers -----

    async fn actor(&self, ctx: &RequestContext) -> Result<UserRef, Error> {
        let user = ctx.principal.require_user()?;
        self.directory.handle(user).await.ok_or_else(|| {
            Error::new(
                ErrorKind::PermissionDenied,
                "this account has no handle for Work".to_owned(),
            )
        })
    }

    async fn ctx(&self, rc: &RequestContext) -> Result<Ctx, Error> {
        let now = self.clock.now();
        Ok(Ctx {
            now,
            today: now.date(),
            actor: Some(self.actor(rc).await?),
        })
    }

    /// Writes the files a command changed, publishes its events, or puts
    /// memory back and fails.
    async fn commit(
        &self,
        st: &mut State,
        backup: State,
        changes: Changes,
        comments: Option<CommentChanges>,
        extra: Vec<(String, String)>,
    ) -> Result<(), Error> {
        let mut writes = extra;
        let mut deletes: BTreeSet<String> = changes.removed_paths.clone();
        for id in &changes.issues {
            if let Some(issue) = st.graph.issue(id) {
                writes.push((issue.path(), issue.to_markdown()?));
            }
        }
        for key in &changes.teams {
            if let Some(team) = st.graph.team(key) {
                writes.push((team.path(), team.to_yaml()?));
            }
        }
        for (team, key) in &changes.cycles {
            if let Some(c) = st.graph.cycles(team).iter().find(|c| c.key == *key) {
                writes.push((c.path(), c.to_yaml()?));
            }
        }
        let mut events = Vec::new();
        if let Some(cc) = comments {
            for id in &cc.threads {
                if let Some(t) = st.comments.thread(id) {
                    writes.push((t.path(), t.to_yaml()?));
                }
            }
            deletes.extend(cc.removed_paths);
            events.extend(cc.events);
        }
        events.extend(changes.events);
        if let Err(e) = self.write_all(&writes, &deletes).await {
            *st = backup;
            return Err(e);
        }
        if !events.is_empty() {
            self.events.publish(events);
        }
        Ok(())
    }

    async fn write_all(
        &self,
        writes: &[(String, String)],
        deletes: &BTreeSet<String>,
    ) -> Result<(), Error> {
        for (path, text) in writes {
            let p = blob_path(path)?;
            let _ = self
                .store
                .put(&p, Bytes::from(text.clone()), Precondition::None)
                .await
                .map_err(store_error)?;
        }
        for path in deletes {
            let p = blob_path(path)?;
            match self.store.delete(&p, Precondition::None).await {
                Ok(()) | Err(StoreError::NotFound(_)) => {}
                Err(e) => return Err(store_error(e)),
            }
        }
        Ok(())
    }

    fn issue_id(graph: &WorkGraph, key: &str) -> Result<IssueId, Error> {
        graph
            .resolve(key)
            .map(|i| i.id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("unknown issue {key}")))
    }

    fn team_key(graph: &WorkGraph, text: &str) -> Result<TeamKey, Error> {
        let key = TeamKey::parse(text)?;
        if graph.team(&key).is_none() {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("unknown team {text}"),
            ));
        }
        Ok(key)
    }

    // ----- queries -----

    pub(crate) async fn list_teams(
        &self,
        _rc: RequestContext,
        _: NoInput,
    ) -> Result<routes::TeamsOut, Error> {
        let st = self.state.lock().await;
        Ok(routes::TeamsOut {
            teams: st.graph.teams().map(TeamOut::of).collect(),
        })
    }

    pub(crate) async fn get_team(&self, _rc: RequestContext, i: TeamRef) -> Result<TeamOut, Error> {
        let st = self.state.lock().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        st.graph
            .team(&key)
            .map(TeamOut::of)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("unknown team {}", i.team)))
    }

    pub(crate) async fn get_issue(
        &self,
        _rc: RequestContext,
        i: IssueRef,
    ) -> Result<IssueOut, Error> {
        let st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let issue = st
            .graph
            .issue(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, i.key.clone()))?;
        Ok(IssueOut::of(issue, &st.graph, self.clock.now(), true))
    }

    pub(crate) async fn query_issues(
        &self,
        rc: RequestContext,
        i: QueryIssues,
    ) -> Result<IssueGroups, Error> {
        let ctx = self.ctx(&rc).await?;
        let me = ctx.actor.clone();
        let st = self.state.lock().await;
        let team = i
            .team
            .as_deref()
            .map(|t| Self::team_key(&st.graph, t))
            .transpose()?;
        let filter = match &i.filter {
            Some(text) => {
                let f: Filter =
                    serde_json::from_str(text).map_err(|e| invalid(&format!("filter: {e}")))?;
                f.check().map_err(|e| invalid(&format!("filter: {e}")))?;
                f
            }
            None => Filter::default(),
        };
        let v = View {
            id: ViewId::generate(),
            name: "Query".to_owned(),
            owner: me
                .clone()
                .unwrap_or_else(|| UserRef::parse("anonymous").unwrap_or_else(|_| unreachable!())),
            team,
            visibility: Visibility::Private,
            layout: i
                .layout
                .as_deref()
                .map(|t| parse_name("layout", t))
                .transpose()?
                .unwrap_or_default(),
            filter,
            group_by: i
                .group_by
                .as_deref()
                .map(|t| parse_name("group_by", t))
                .transpose()?
                .unwrap_or(view::GroupBy::None),
            order_by: i
                .order_by
                .as_deref()
                .map(|t| parse_name("order_by", t))
                .transpose()?
                .unwrap_or_default(),
            show_closed: i
                .show_closed
                .as_deref()
                .map(|t| parse_name("show_closed", t))
                .transpose()?
                .unwrap_or_default(),
            show_sub_issues: true,
        };
        let groups = view::run(&v, &st.graph, &ctx, me.as_ref());
        Ok(groups_out(&st.graph, groups, i.limit, ctx.now))
    }

    pub(crate) async fn inbox(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<IssueGroups, Error> {
        let st = self.state.lock().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let now = self.clock.now();
        let issues = st
            .graph
            .inbox(&key, now)
            .into_iter()
            .map(|issue| IssueOut::of(issue, &st.graph, now, false))
            .collect();
        Ok(IssueGroups {
            groups: vec![GroupOut { key: None, issues }],
        })
    }

    pub(crate) async fn my_issues(
        &self,
        rc: RequestContext,
        i: MyIssues,
    ) -> Result<MyIssuesOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let me = ctx.actor.clone().ok_or_else(|| invalid("no user"))?;
        let tab: Tab = i
            .tab
            .as_deref()
            .map(|t| parse_name("tab", t))
            .transpose()?
            .unwrap_or(Tab::Assigned);
        let st = self.state.lock().await;
        let issues = my_issues(&st.graph, &me, tab, &ctx, &[])
            .into_iter()
            .map(|(focus, issue)| FocusedIssue {
                focus: api::name_of(&focus),
                issue: IssueOut::of(issue, &st.graph, ctx.now, false),
            })
            .collect();
        Ok(MyIssuesOut { issues })
    }

    pub(crate) async fn list_cycles(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<CyclesOut, Error> {
        let st = self.state.lock().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let today = self.clock.now().date();
        Ok(CyclesOut {
            cycles: st.graph.cycles(&key).iter().map(CycleOut::of).collect(),
            velocity: st.graph.velocity(&key, today),
        })
    }

    pub(crate) async fn list_projects(
        &self,
        _rc: RequestContext,
        _: NoInput,
    ) -> Result<routes::ProjectsOut, Error> {
        let st = self.state.lock().await;
        Ok(routes::ProjectsOut {
            projects: st
                .graph
                .projects()
                .map(|p| ProjectOut::of(p, &st.graph.project_progress(&p.id)))
                .collect(),
        })
    }

    pub(crate) async fn get_project(
        &self,
        _rc: RequestContext,
        i: ProjectRef,
    ) -> Result<ProjectOut, Error> {
        let st = self.state.lock().await;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        let p = st
            .graph
            .project(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("unknown project {id}")))?;
        Ok(ProjectOut::of(p, &st.graph.project_progress(&id)))
    }

    pub(crate) async fn list_comments(
        &self,
        _rc: RequestContext,
        i: ListComments,
    ) -> Result<ThreadsOut, Error> {
        let st = self.state.lock().await;
        let target = st.graph.comment_target(&i.target)?;
        let block = match &target {
            Target::Doc { block, .. } => block.clone(),
            Target::Issue { .. } => None,
        };
        let threads = st
            .comments
            .on(&target.object())
            .into_iter()
            .filter(|t| match (&block, &t.target) {
                (
                    Some(b),
                    Target::Doc {
                        block: Some(tb), ..
                    },
                ) => b == tb,
                (Some(_), _) => false,
                (None, _) => true,
            })
            .map(|t| ThreadOut::of(t, &st.graph))
            .collect();
        Ok(ThreadsOut { threads })
    }

    pub(crate) async fn my_assignments(
        &self,
        rc: RequestContext,
        _: NoInput,
    ) -> Result<AssignmentsOut, Error> {
        let me = self.actor(&rc).await?;
        let st = self.state.lock().await;
        Ok(AssignmentsOut {
            assignments: st
                .comments
                .assignments(&me)
                .iter()
                .map(|a| AssignmentOut::of(a, &st.graph))
                .collect(),
        })
    }

    // ----- issue commands -----

    pub(crate) async fn create_issue(
        &self,
        rc: RequestContext,
        i: CreateIssue,
    ) -> Result<CreatedIssue, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let team = Self::team_key(&st.graph, &i.team)?;
        let mut draft = NewIssue::new(IssueId::generate(), team, &i.title);
        draft.body = i.description;
        draft.template = i.template;
        draft.state = i.state.as_deref().map(StatusKey::parse).transpose()?;
        draft.priority = i.priority.map(priority).transpose()?;
        draft.estimate = i.estimate.as_deref().map(estimate).transpose()?;
        draft.assignee = i.assignee.as_deref().map(UserRef::parse).transpose()?;
        draft.labels = i.labels;
        draft.project = i
            .project
            .as_deref()
            .map(|p| parse_id("project", p))
            .transpose()?;
        draft.parent = i
            .parent
            .as_deref()
            .map(|k| Self::issue_id(&st.graph, k))
            .transpose()?;
        draft.due = i.due.as_deref().map(date).transpose()?;
        draft.intake = i.intake;
        let source = i
            .source_block
            .as_deref()
            .map(str::parse::<BlockRef>)
            .transpose()?;
        if let Some(at) = &source {
            draft
                .links
                .push(doclink::source_link(at, i.quote.as_deref()));
        }
        let id = draft.id;
        let backup = st.clone();
        let changes = st.graph.create_issue(draft, &ctx)?;
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        let issue = st
            .graph
            .issue(&id)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "issue vanished".to_owned()))?;
        Ok(CreatedIssue {
            chip: source.map(|at| ChipOut {
                doc: at.doc.to_string(),
                block: at.block,
                key: issue.key().to_string(),
            }),
            issue: IssueOut::of(issue, &st.graph, ctx.now, true),
        })
    }

    pub(crate) async fn update_issue(
        &self,
        rc: RequestContext,
        i: UpdateIssue,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let me = ctx.actor.clone();
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let current = st
            .graph
            .issue(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, i.key.clone()))?;
        let patch = patch_from(&st.graph, current, i, me)?;
        let backup = st.clone();
        let changes = st.graph.update_issue(&id, patch, &ctx)?;
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn move_issue(
        &self,
        rc: RequestContext,
        i: MoveIssue,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let to = Self::team_key(&st.graph, &i.to)?;
        let backup = st.clone();
        let changes = st.graph.move_issue(&id, &to, &ctx)?;
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn triage_issue(
        &self,
        rc: RequestContext,
        i: TriageIssue,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let action = match i.action.as_str() {
            "accept" => TriageAction::Accept {
                status: i.state.as_deref().map(StatusKey::parse).transpose()?,
            },
            "decline" => TriageAction::Decline { reason: i.reason },
            "duplicate" => TriageAction::Duplicate {
                of: Self::issue_id(
                    &st.graph,
                    i.duplicate_of
                        .as_deref()
                        .ok_or_else(|| invalid("duplicate needs duplicate_of"))?,
                )?,
            },
            "snooze" => TriageAction::Snooze {
                until: OffsetDateTime::parse(
                    i.until
                        .as_deref()
                        .ok_or_else(|| invalid("snooze needs until"))?,
                    &Rfc3339,
                )
                .map_err(|_| invalid("until must be an RFC 3339 time"))?,
            },
            other => return Err(invalid(&format!("unknown triage action {other:?}"))),
        };
        let backup = st.clone();
        let changes = st.graph.triage(&id, &action, &ctx)?;
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    fn issue_out(&self, st: &State, id: &IssueId, now: OffsetDateTime) -> Result<IssueOut, Error> {
        let _ = self;
        st.graph
            .issue(id)
            .map(|issue| IssueOut::of(issue, &st.graph, now, true))
            .ok_or_else(|| Error::new(ErrorKind::NotFound, id.to_string()))
    }

    // ----- teams and projects -----

    pub(crate) async fn create_team(
        &self,
        rc: RequestContext,
        i: CreateTeam,
    ) -> Result<TeamOut, Error> {
        let _ = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let mut team = Team::new(TeamId::generate(), TeamKey::parse(&i.key)?, &i.name)?;
        team.lead = i.lead.as_deref().map(UserRef::parse).transpose()?;
        team.members = i
            .members
            .iter()
            .map(|m| UserRef::parse(m))
            .collect::<Result<_, _>>()?;
        let backup = st.clone();
        let changes = match add_team(&mut st.graph, team.clone(), self.clock.now().date()) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        Ok(TeamOut::of(&team))
    }

    pub(crate) async fn put_team(&self, rc: RequestContext, i: PutTeam) -> Result<TeamOut, Error> {
        let _ = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let path = format!("teams/{key}/team.yaml");
        let team = Team::from_yaml(&path, &i.yaml)?;
        if team.key != key {
            return Err(invalid(
                "the file's key must match the team in the path; move issues to rename a team",
            ));
        }
        let backup = st.clone();
        let changes = st.graph.update_team(team)?;
        self.commit(&mut st, backup, changes, None, Vec::new())
            .await?;
        st.graph
            .team(&key)
            .map(TeamOut::of)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, key.to_string()))
    }

    pub(crate) async fn put_project(
        &self,
        rc: RequestContext,
        i: PutProject,
    ) -> Result<ProjectOut, Error> {
        let _ = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        let path = st.project_paths.get(&id).cloned();
        let project = Project::from_markdown(
            path.as_deref().unwrap_or("projects/new/project.md"),
            &i.markdown,
        )?;
        if project.id != id {
            return Err(invalid("the file's id must match project_id"));
        }
        let path = path.unwrap_or_else(|| project.default_path());
        let text = project.to_markdown()?;
        let backup = st.clone();
        let changes = st.graph.upsert_project(project)?;
        let _ = st.project_paths.insert(id, path.clone());
        self.commit(&mut st, backup, changes, None, vec![(path, text)])
            .await?;
        let p = st
            .graph
            .project(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, id.to_string()))?;
        Ok(ProjectOut::of(p, &st.graph.project_progress(&id)))
    }

    // ----- comments -----

    pub(crate) async fn add_comment(
        &self,
        rc: RequestContext,
        i: AddComment,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let reply_to = i
            .reply_to
            .as_deref()
            .map(|c| parse_id::<CommentId>("reply_to", c))
            .transpose()?;
        let target = match (&i.target, reply_to) {
            (Some(t), None) => Some(st.graph.comment_target(t)?),
            (None, Some(_)) => None,
            _ => {
                return Err(invalid(
                    "give either a target (new thread) or reply_to (reply)",
                ));
            }
        };
        let thread_target = match (&target, reply_to) {
            (Some(t), _) => Some(t.clone()),
            (None, Some(c)) => st.comments.thread_of(&c).map(|t| t.target.clone()),
            (None, None) => None,
        };
        // Followers of the target hear about every comment.
        let audience = thread_target
            .and_then(|t| t.issue())
            .and_then(|id| st.graph.issue(&id))
            .map(|issue| issue.subscribers.clone())
            .unwrap_or_default();
        let id = CommentId::generate();
        let draft = NewComment {
            id,
            target,
            reply_to,
            quote: i.quote,
            body: i.body,
            assign_to: i
                .assign_to
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(UserRef::parse)
                .transpose()?,
            authored_by: if i.by_model {
                AuthoredBy::Model
            } else {
                AuthoredBy::Person
            },
        };
        let backup = st.clone();
        let cc = st.comments.add(draft, &audience, &ctx)?;
        let changes = match st.graph.apply_comment(&cc, &ctx) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        self.commit(&mut st, backup, changes, Some(cc), Vec::new())
            .await?;
        Ok(thread_result(&st, &id))
    }

    pub(crate) async fn edit_comment(
        &self,
        rc: RequestContext,
        i: EditComment,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("comment_id", &i.comment_id)?;
        self.comment_command(ctx, &id, |c, ctx| c.edit(&id, &i.body, ctx))
            .await
    }

    pub(crate) async fn delete_comment(
        &self,
        rc: RequestContext,
        i: CommentRef,
    ) -> Result<CommentResult, Error> {
        let moderator = rc.principal.scopes().allows(Scope::Admin);
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("comment_id", &i.comment_id)?;
        self.comment_command(ctx, &id, |c, ctx| c.delete(&id, moderator, ctx))
            .await
    }

    pub(crate) async fn react(&self, rc: RequestContext, i: React) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("comment_id", &i.comment_id)?;
        self.comment_command(ctx, &id, |c, ctx| c.react(&id, &i.emoji, i.on, ctx))
            .await
    }

    pub(crate) async fn resolve_thread(
        &self,
        rc: RequestContext,
        i: ResolveThread,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("thread_id", &i.thread_id)?;
        self.comment_command(ctx, &id, |c, ctx| c.resolve(&id, i.resolved, ctx))
            .await
    }

    pub(crate) async fn assign_thread(
        &self,
        rc: RequestContext,
        i: AssignThread,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("thread_id", &i.thread_id)?;
        let to = i
            .assignee
            .as_deref()
            .filter(|a| !a.is_empty())
            .map(UserRef::parse)
            .transpose()?;
        self.comment_command(ctx, &id, |c, ctx| c.assign(&id, to, ctx))
            .await
    }

    async fn comment_command<F>(
        &self,
        ctx: Ctx,
        id: &CommentId,
        f: F,
    ) -> Result<CommentResult, Error>
    where
        F: FnOnce(&mut Comments, &Ctx) -> vellum_work::Result<CommentChanges>,
    {
        let mut st = self.state.lock().await;
        let thread = st.comments.thread_of(id).map(|t| t.id);
        let backup = st.clone();
        let cc = f(&mut st.comments, &ctx)?;
        let changes = match st.graph.apply_comment(&cc, &ctx) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        self.commit(&mut st, backup, changes, Some(cc), Vec::new())
            .await?;
        Ok(thread.map_or(CommentResult { thread: None }, |t| thread_result(&st, &t)))
    }
}

/// Adds a team and pre-creates its cycles.
fn add_team(graph: &mut WorkGraph, team: Team, today: Date) -> vellum_work::Result<Changes> {
    let key = team.key.clone();
    graph.insert_team(team)?;
    let mut changes = graph.ensure_cycles(today)?;
    let _ = changes.teams.insert(key);
    Ok(changes)
}

fn thread_result(st: &State, id: &CommentId) -> CommentResult {
    CommentResult {
        thread: st
            .comments
            .thread_of(id)
            .map(|t| ThreadOut::of(t, &st.graph)),
    }
}

fn groups_out(
    graph: &WorkGraph,
    groups: Vec<view::Group>,
    limit: Option<usize>,
    now: OffsetDateTime,
) -> IssueGroups {
    let mut left = limit.unwrap_or(DEFAULT_LIST).min(MAX_LIST);
    let groups = groups
        .into_iter()
        .map(|g| {
            let issues: Vec<IssueOut> = g
                .issues
                .iter()
                .take(left)
                .filter_map(|id| graph.issue(id))
                .map(|issue| IssueOut::of(issue, graph, now, false))
                .collect();
            left -= issues.len();
            GroupOut { key: g.key, issues }
        })
        .collect();
    IssueGroups { groups }
}

/// Builds the engine patch from an API update.
fn patch_from(
    graph: &WorkGraph,
    current: &Issue,
    i: UpdateIssue,
    me: Option<UserRef>,
) -> Result<IssuePatch, Error> {
    let clearable = |v: Option<String>| v.map(|s| (!s.is_empty()).then_some(s));
    let mut patch = IssuePatch {
        title: i.title,
        body: i.description,
        state: i.state.as_deref().map(StatusKey::parse).transpose()?,
        priority: i.priority.map(priority).transpose()?,
        estimate: clearable(i.estimate)
            .map(|v| v.as_deref().map(estimate).transpose())
            .transpose()?,
        assignee: clearable(i.assignee)
            .map(|v| v.as_deref().map(UserRef::parse).transpose())
            .transpose()?,
        project: clearable(i.project)
            .map(|v| {
                v.as_deref()
                    .map(|p| parse_id::<ProjectId>("project", p))
                    .transpose()
            })
            .transpose()?,
        cycle: clearable(i.cycle)
            .map(|v| v.as_deref().map(CycleKey::parse).transpose())
            .transpose()?,
        parent: clearable(i.parent)
            .map(|v| {
                v.as_deref()
                    .map(|k| WorkService::issue_id(graph, k))
                    .transpose()
            })
            .transpose()?,
        due: clearable(i.due)
            .map(|v| v.as_deref().map(date).transpose())
            .transpose()?,
        ..IssuePatch::default()
    };
    if !i.add_labels.is_empty() || !i.remove_labels.is_empty() {
        let mut labels: Vec<String> = current
            .labels
            .iter()
            .filter(|l| !i.remove_labels.contains(l))
            .cloned()
            .collect();
        for l in i.add_labels {
            if !labels.contains(&l) {
                labels.push(l);
            }
        }
        patch.labels = Some(labels);
    }
    let relation = |r: &api::RelationIn| -> Result<Relation, Error> {
        Ok(Relation {
            kind: parse_name("relation kind", &r.kind)?,
            issue: WorkService::issue_id(graph, &r.issue)?,
        })
    };
    patch.add_relations = i
        .add_relations
        .iter()
        .map(relation)
        .collect::<Result<_, _>>()?;
    patch.remove_relations = i
        .remove_relations
        .iter()
        .map(relation)
        .collect::<Result<_, _>>()?;
    match (i.subscribe, me) {
        (Some(true), Some(me)) => patch.subscribe = vec![me],
        (Some(false), Some(me)) => patch.unsubscribe = vec![me],
        _ => {}
    }
    Ok(patch)
}

fn priority(n: u8) -> Result<Priority, Error> {
    Priority::from_number(n).ok_or_else(|| invalid("priority must be 0 to 4"))
}

fn estimate(text: &str) -> Result<Estimate, Error> {
    Estimate::from_text(text)
        .ok_or_else(|| invalid(&format!("estimate {text:?} is not a number or size")))
}

fn date(text: &str) -> Result<Date, Error> {
    Date::parse(text, format_description!("[year]-[month]-[day]"))
        .map_err(|_| invalid(&format!("date {text:?} must be YYYY-MM-DD")))
}

fn parse_id<T: std::str::FromStr>(field: &str, text: &str) -> Result<T, Error> {
    text.parse::<T>()
        .map_err(|_| invalid(&format!("{field}: {text:?} is not a valid ID")))
}

fn blob_path(path: &str) -> Result<BlobPath, Error> {
    BlobPath::new(path).map_err(store_error)
}

fn store_error(e: StoreError) -> Error {
    let kind = match &e {
        StoreError::NotFound(_) => ErrorKind::NotFound,
        StoreError::AlreadyExists(_) | StoreError::PreconditionFailed { .. } => ErrorKind::Conflict,
        StoreError::InvalidPath { .. } => ErrorKind::InvalidInput,
        _ => ErrorKind::Unavailable,
    };
    Error::new(kind, format!("Work storage: {e}")).with_source(e)
}

/// Lists the blob paths under `dir`; a missing directory is empty.
async fn list(store: &dyn BlobStore, dir: &str) -> Result<Vec<String>, Error> {
    let dir = blob_path(dir)?;
    let mut out = Vec::new();
    let mut stream = store.list(Some(&dir));
    while let Some(entry) = stream.next().await {
        match entry {
            Ok(e) => out.push(e.path.as_str().to_owned()),
            Err(StoreError::NotFound(_)) => break,
            Err(e) => return Err(store_error(e)),
        }
    }
    Ok(out)
}

/// Loads one file into `state`; `Ok(false)` for files Work does not own.
async fn load_one(store: &dyn BlobStore, state: &mut State, path: &str) -> Result<bool, Error> {
    let kind = classify(path);
    if kind == Kind::Other {
        return Ok(false);
    }
    let blob = store.get(&blob_path(path)?).await.map_err(store_error)?;
    let text = std::str::from_utf8(&blob.bytes).map_err(|_| invalid("not UTF-8"))?;
    match kind {
        Kind::Team => state.graph.insert_team(Team::from_yaml(path, text)?)?,
        Kind::Issue => state.graph.load_issue(Issue::from_markdown(path, text)?)?,
        Kind::Cycle => state.graph.load_cycle(Cycle::from_yaml(path, text)?)?,
        Kind::Project => {
            let p = Project::from_markdown(path, text)?;
            let _ = state.project_paths.insert(p.id, path.to_owned());
            let _ = state.graph.upsert_project(p)?;
        }
        Kind::Thread => state.comments.load(Thread::from_yaml(path, text)?)?,
        Kind::Other => {}
    }
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Team,
    Issue,
    Cycle,
    Project,
    Thread,
    Other,
}

fn has_ext(file: &str, ext: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn classify(path: &str) -> Kind {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["teams", _, "team.yaml"] => Kind::Team,
        ["teams", _, "issues", f] if has_ext(f, "md") => Kind::Issue,
        ["teams", _, "cycles", f] if has_ext(f, "yaml") => Kind::Cycle,
        ["projects", _, "project.md"] => Kind::Project,
        [".comments", _, f] if has_ext(f, "yaml") => Kind::Thread,
        _ => Kind::Other,
    }
}

/// Every operation the Work service serves.
pub fn operations(svc: &Arc<WorkService>) -> Vec<Operation> {
    routes::all(svc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_work_files() {
        assert_eq!(classify("teams/ENG/team.yaml"), Kind::Team);
        assert_eq!(classify("teams/ENG/issues/ENG-1.md"), Kind::Issue);
        assert_eq!(classify("teams/ENG/cycles/2026-C44.yaml"), Kind::Cycle);
        assert_eq!(classify("projects/gateway/project.md"), Kind::Project);
        assert_eq!(classify(".comments/01JB/01JD.yaml"), Kind::Thread);
        assert_eq!(
            classify("projects/gateway/updates/2026-11-02-abc.md"),
            Kind::Other
        );
        assert_eq!(classify("teams/ENG/notes.md"), Kind::Other);
    }

    #[test]
    fn dates_and_scales_are_checked() {
        assert!(date("2026-11-02").is_ok());
        assert!(date("02/11/2026").is_err());
        assert!(priority(5).is_err());
        assert!(estimate("huge").is_err());
    }
}
