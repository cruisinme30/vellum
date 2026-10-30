// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`WorkService`]: the Work engine behind the gateway (D135–D137).

#![allow(
    clippy::significant_drop_tightening,
    reason = "each command holds the state lock until its files are written, \
        so commands never interleave"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::StreamExt;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use tokio::sync::{Mutex, MutexGuard};
use tracing::{info, instrument, warn};
use vellum_contracts::{Operation, RequestContext, Scope};
use vellum_kernel::{CommentId, Error, ErrorKind, TeamId};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};
use vellum_work::comment::{
    AuthoredBy, CommentChanges, Comments, NewComment, Target, Thread,
};
use vellum_work::cycle::{Cycle, CycleKey};
use vellum_work::doclink;
use vellum_work::filter::Filter;
use vellum_work::my_issues::{Tab, my_issues};
use vellum_work::project::Project;
use vellum_work::view::{self, View, Visibility};
use vellum_work::{
    BlockRef, Changes, Ctx, Estimate, Issue, IssueId, IssuePatch, MilestoneId,
    NewIssue, Priority, ProjectId, Relation, Repeat, StatusKey, Team, TeamKey,
    TriageAction, UserRef, ViewId, WorkGraph,
};

use crate::api::{
    self, AddComment, AssignThread, AssignmentOut, AssignmentsOut, ChipOut,
    CommentRef, CommentResult, CreateIssue, CreateTeam, CreatedIssue, CycleOut,
    CyclesOut, EditComment, FocusedIssue, GroupOut, IssueGroups, IssueOut,
    IssueRef, ListComments, ListTeams, MoveIssue, MyIssues, MyIssuesOut,
    NoInput, ProjectOut, ProjectRef, PutProject, PutTeam, QueryIssues, React,
    ResolveThread, TeamOut, TeamRef, ThreadOut, ThreadsOut, TriageIssue,
    UpdateIssue, invalid, parse_name,
};
use crate::api::{
    Done, ListViews, PutView, ViewIn, ViewOut, ViewRef, ViewsOut,
};
use crate::journal::{self, CatchUp};
use crate::ports::{Clock, Directory, EventSink};
use crate::routes;

pub mod embed;
pub mod embeds;
pub mod insights;
pub mod live;
pub mod planning;
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
    /// Saved views, stored as `views/<id>.yaml`.
    pub(crate) views: BTreeMap<ViewId, View>,
    /// Project updates and initiatives (D223).
    pub(crate) planning: planning::Planning,
    /// The journal revision this copy reflects (see [`crate::journal`]).
    pub(crate) seen: u64,
    /// When this copy last compared itself with the journal for a read.
    checked: Option<Instant>,
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
    /// Names this replica in journal entries.
    writer: String,
    /// How stale a read may be before it checks the journal again.
    read_fresh: Duration,
}

/// How long reads may serve this replica's copy before checking for other
/// replicas' writes. Keeps cross-replica staleness well under a second.
pub const READ_FRESH: Duration = Duration::from_millis(250);

/// How long a writer waits for another replica's pending write.
const WAIT_FOR_WRITER: Duration = Duration::from_secs(10);

/// How many times a command is retried after losing a race.
pub const ATTEMPTS: usize = 8;

/// The message of the internal error that asks for a retry.
const RACE: &str = "another replica changed Work first; retrying";

/// Whether `e` is a lost race that the command should simply run again.
pub fn is_race(e: &Error) -> bool {
    e.kind() == ErrorKind::Conflict && e.message() == RACE
}

/// Runs `f` until it does not lose a race to another replica.
///
/// # Errors
///
/// The command's own error, or `conflict` after too many lost races.
pub async fn retrying<T, F, Fut>(mut f: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    let mut last = None;
    for _ in 0..ATTEMPTS {
        match f().await {
            Err(e) if is_race(&e) => last = Some(e),
            other => return other,
        }
    }
    Err(last.map_or_else(
        || Error::new(ErrorKind::Conflict, RACE),
        |_| {
            Error::new(
                ErrorKind::Conflict,
                "Work is busy on another replica; try again",
            )
        },
    ))
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
        let head = journal::read(store.as_ref()).await?;
        let (mut state, report) = load_all(store.as_ref()).await?;
        state.seen = head.journal.readable_rev(journal::now_ms());
        info!(
            loaded = report.loaded,
            skipped = report.skipped,
            rev = state.seen,
            "Work loaded"
        );
        let service = Self {
            store,
            directory,
            events,
            clock,
            state: Mutex::new(state),
            writer: writer_name(),
            read_fresh: READ_FRESH,
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
        retrying(|| self.run_daily_once()).await
    }

    async fn run_daily_once(&self) -> Result<(), Error> {
        let now = self.clock.now();
        let mut st = self.write_state().await?;
        let backup = st.clone();
        let mut changes = st.graph.daily(now, now.date())?;
        let sla = st.graph.sla_tick(now);
        changes.issues.extend(sla.issues);
        changes.events.extend(sla.events);
        let recur = st.graph.recur(
            now,
            |t| planning::team_today(t, now),
            IssueId::generate,
        )?;
        changes.issues.extend(recur.issues);
        changes.teams.extend(recur.teams);
        changes.cycles.extend(recur.cycles);
        changes.events.extend(recur.events);
        self.commit(&mut st, backup, changes, None, Vec::new(), None)
            .await
    }

    /// Files the next issue of each repeating issue whose due date has
    /// passed in its team's time zone (D224). Cheap when nothing is due;
    /// `vellum serve` runs it every few minutes on one replica.
    ///
    /// # Errors
    ///
    /// Returns `unavailable` if the files cannot be written.
    #[instrument(skip_all)]
    pub async fn run_recurring(&self) -> Result<(), Error> {
        retrying(|| async {
            let now = self.clock.now();
            let mut st = self.write_state().await?;
            let backup = st.clone();
            let changes = st.graph.recur(
                now,
                |t| planning::team_today(t, now),
                IssueId::generate,
            )?;
            self.commit(&mut st, backup, changes, None, Vec::new(), None)
                .await
        })
        .await
    }

    // ----- replicas -----

    /// The state for a query: this replica's copy, checked against the
    /// journal at most every [`READ_FRESH`]. If the journal cannot be read
    /// the copy is served as it is.
    pub(crate) async fn read_state(&self) -> MutexGuard<'_, State> {
        let mut st = self.state.lock().await;
        let due = st.checked.is_none_or(|t| t.elapsed() >= self.read_fresh);
        if due {
            if let Err(e) = self.sync(&mut st, false).await {
                warn!(error = %e, "Work served from this replica's copy");
            }
            st.checked = Some(Instant::now());
        }
        st
    }

    /// The state for a command: waits out another replica's pending write
    /// and loads everything committed before it.
    ///
    /// # Errors
    ///
    /// `unavailable` when the journal cannot be read, or another replica
    /// holds Work for longer than ten seconds.
    pub(crate) async fn write_state(
        &self,
    ) -> Result<MutexGuard<'_, State>, Error> {
        let mut st = self.state.lock().await;
        self.sync(&mut st, true).await?;
        st.checked = Some(Instant::now());
        Ok(st)
    }

    /// Loads what other replicas committed since `st.seen`.
    async fn sync(&self, st: &mut State, writing: bool) -> Result<(), Error> {
        let started = Instant::now();
        let head = loop {
            let head = journal::read(self.store.as_ref()).await?;
            if !writing || head.journal.blocking(journal::now_ms()).is_none() {
                break head;
            }
            if started.elapsed() > WAIT_FOR_WRITER {
                return Err(Error::new(
                    ErrorKind::Unavailable,
                    "another replica is still writing Work; try again",
                ));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let target = head.journal.readable_rev(journal::now_ms());
        match head.journal.catch_up(st.seen, target) {
            CatchUp::Current => Ok(()),
            CatchUp::Paths { paths, rev } => {
                let mut next = st.clone();
                match apply_paths(self.store.as_ref(), &mut next, &paths).await
                {
                    Ok(()) => {
                        next.seen = rev;
                        *st = next;
                        Ok(())
                    }
                    Err(e) => {
                        info!(reason = %e, "Work rebuilt from files");
                        self.reload(st, rev).await
                    }
                }
            }
            CatchUp::Reload { rev } => self.reload(st, rev).await,
        }
    }

    async fn reload(&self, st: &mut State, rev: u64) -> Result<(), Error> {
        let (mut fresh, report) = load_all(self.store.as_ref()).await?;
        fresh.seen = rev;
        fresh.checked = st.checked;
        info!(
            loaded = report.loaded,
            rev, "Work reloaded for another replica's changes"
        );
        *st = fresh;
        Ok(())
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

    /// Writes the files a command changed, publishes its events with the
    /// person or bot who made the change (`None` for scheduled work), or
    /// puts memory back and fails.
    async fn commit(
        &self,
        st: &mut State,
        backup: State,
        changes: Changes,
        comments: Option<CommentChanges>,
        extra: Vec<(String, String)>,
        actor: Option<&UserRef>,
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
            if let Some(c) =
                st.graph.cycles(team).iter().find(|c| c.key == *key)
            {
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
        let mut paths: Vec<String> =
            writes.iter().map(|(p, _)| p.clone()).collect();
        paths.extend(deletes.iter().cloned());
        if !paths.is_empty() {
            // Claim the next revision; losing the claim means another
            // replica committed first, so the command runs again on fresh
            // state.
            let head = journal::read(self.store.as_ref()).await;
            let head = match head {
                Ok(h) => h,
                Err(e) => {
                    *st = backup;
                    return Err(e);
                }
            };
            let now = journal::now_ms();
            if head.journal.rev != st.seen
                || head.journal.blocking(now).is_some()
            {
                *st = backup;
                return Err(Error::new(ErrorKind::Conflict, RACE));
            }
            let claimed = head.journal.claim(paths, &self.writer, now);
            let etag = match journal::write(
                self.store.as_ref(),
                &claimed,
                head.etag.as_ref(),
            )
            .await
            {
                Ok(Some(etag)) => etag,
                Ok(None) => {
                    *st = backup;
                    return Err(Error::new(ErrorKind::Conflict, RACE));
                }
                Err(e) => {
                    *st = backup;
                    return Err(e);
                }
            };
            let written = self.write_all(&writes, &deletes).await;
            // Mark the entry done even when a write failed: the files that
            // did land are what every replica, this one included, loads.
            match journal::write(
                self.store.as_ref(),
                &claimed.finish(claimed.rev),
                Some(&etag),
            )
            .await
            {
                Ok(Some(_)) => {}
                Ok(None) => {
                    warn!(rev = claimed.rev, "a Work write outlived its lease")
                }
                Err(e) => {
                    warn!(rev = claimed.rev, error = %e, "Work journal entry \
                        left pending")
                }
            }
            if let Err(e) = written {
                *st = backup;
                return Err(e);
            }
            st.seen = claimed.rev;
        }
        if !events.is_empty() {
            self.events.publish_by(actor, events);
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
        graph.resolve(key).map(|i| i.id).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("unknown issue {key}"))
        })
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
        i: ListTeams,
    ) -> Result<routes::TeamsOut, Error> {
        let st = self.read_state().await;
        Ok(routes::TeamsOut {
            teams: st
                .graph
                .teams()
                .filter(|t| i.include_archived || !t.archived)
                .map(TeamOut::of)
                .collect(),
        })
    }

    pub(crate) async fn get_team(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<TeamOut, Error> {
        let st = self.read_state().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        st.graph.team(&key).map(TeamOut::of).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("unknown team {}", i.team))
        })
    }

    pub(crate) async fn get_team_settings(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<crate::api::TeamSettingsOut, Error> {
        let st = self.read_state().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let team = st.graph.team(&key).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("unknown team {}", i.team))
        })?;
        let settings = serde_json::to_value(team).map_err(|e| {
            Error::new(ErrorKind::Internal, format!("team {key}: {e}"))
        })?;
        Ok(crate::api::TeamSettingsOut {
            settings,
            yaml: team.to_yaml()?,
        })
    }

    pub(crate) async fn get_issue(
        &self,
        _rc: RequestContext,
        i: IssueRef,
    ) -> Result<IssueOut, Error> {
        let st = self.read_state().await;
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
        let st = self.read_state().await;
        let team = i
            .team
            .as_deref()
            .map(|t| Self::team_key(&st.graph, t))
            .transpose()?;
        let filter = match &i.filter {
            Some(text) => {
                let f: Filter = serde_json::from_str(text)
                    .map_err(|e| invalid(&format!("filter: {e}")))?;
                f.check().map_err(|e| invalid(&format!("filter: {e}")))?;
                f
            }
            None => Filter::default(),
        };
        let v = View {
            id: ViewId::generate(),
            name: "Query".to_owned(),
            owner: me.clone().unwrap_or_else(|| {
                UserRef::parse("anonymous").unwrap_or_else(|_| unreachable!())
            }),
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
        let st = self.read_state().await;
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
        let st = self.read_state().await;
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
        let st = self.read_state().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let today = self.clock.now().date();
        Ok(CyclesOut {
            cycles: st.graph.cycles(&key).iter().map(CycleOut::of).collect(),
            velocity: st.graph.velocity(&key, today),
        })
    }

    // ----- saved views -----

    pub(crate) async fn list_views(
        &self,
        rc: RequestContext,
        i: ListViews,
    ) -> Result<ViewsOut, Error> {
        let me = self.actor(&rc).await?;
        let st = self.read_state().await;
        let team = i
            .team
            .as_deref()
            .map(|t| Self::team_key(&st.graph, t))
            .transpose()?;
        let member_of = |key: &TeamKey| {
            st.graph.team(key).is_some_and(|t| t.is_member(&me))
        };
        let mut views: Vec<&View> = st
            .views
            .values()
            .filter(|v| v.visible_to(&me, member_of))
            .filter(|v| team.is_none() || v.team.is_none() || v.team == team)
            .collect();
        views.sort_by_key(|v| v.name.to_lowercase());
        Ok(ViewsOut {
            views: views.into_iter().map(ViewOut::of).collect(),
        })
    }

    pub(crate) async fn create_view(
        &self,
        rc: RequestContext,
        i: ViewIn,
    ) -> Result<ViewOut, Error> {
        self.save_view(rc, ViewId::generate(), i, true).await
    }

    pub(crate) async fn put_view(
        &self,
        rc: RequestContext,
        i: PutView,
    ) -> Result<ViewOut, Error> {
        let id = parse_id::<ViewId>("view_id", &i.view_id)?;
        self.save_view(rc, id, i.settings(), false).await
    }

    async fn save_view(
        &self,
        rc: RequestContext,
        id: ViewId,
        i: ViewIn,
        new: bool,
    ) -> Result<ViewOut, Error> {
        let me = self.actor(&rc).await?;
        if i.filter
            .as_ref()
            .is_some_and(|f| f.len() > MAX_VIEW_FILTER_BYTES)
        {
            return Err(invalid("filter: at most 16 KiB of JSON"));
        }
        let mut st = self.write_state().await?;
        if new {
            let owned = st.views.values().filter(|v| v.owner == me).count();
            if owned >= MAX_VIEWS_PER_PERSON {
                return Err(invalid(
                    "you have 200 saved views; delete one to save another",
                ));
            }
        } else {
            let _ = Self::own_view(&st, &id, &me)?;
        }
        let name = i.name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(invalid("name: 1 to 120 characters"));
        }
        let team = i
            .team
            .as_deref()
            .map(|t| Self::team_key(&st.graph, t))
            .transpose()?;
        let filter = match i.filter.as_deref() {
            Some(text) => serde_json::from_str(text)
                .map_err(|e| invalid(&format!("filter: {e}")))?,
            None => Filter::default(),
        };
        let view = View {
            id,
            name: name.to_owned(),
            owner: me.clone(),
            team,
            visibility: pick("visibility", i.visibility.as_deref())?
                .unwrap_or_default(),
            layout: pick("layout", i.layout.as_deref())?.unwrap_or_default(),
            filter,
            group_by: pick("group_by", i.group_by.as_deref())?
                .unwrap_or_default(),
            order_by: pick("order_by", i.order_by.as_deref())?
                .unwrap_or_default(),
            show_closed: pick("show_closed", i.show_closed.as_deref())?
                .unwrap_or_default(),
            show_sub_issues: i.show_sub_issues.unwrap_or(true),
        };
        view.check().map_err(Error::from)?;
        let text = view.to_yaml().map_err(Error::from)?;
        let backup = st.clone();
        let _ = st.views.insert(id, view.clone());
        self.commit(
            &mut st,
            backup,
            Changes::default(),
            None,
            vec![(view.path(), text)],
            Some(&me),
        )
        .await?;
        Ok(ViewOut::of(&view))
    }

    pub(crate) async fn delete_view(
        &self,
        rc: RequestContext,
        i: ViewRef,
    ) -> Result<Done, Error> {
        let me = self.actor(&rc).await?;
        let id = parse_id::<ViewId>("view_id", &i.view_id)?;
        let mut st = self.write_state().await?;
        let path = Self::own_view(&st, &id, &me)?.path();
        let backup = st.clone();
        let _ = st.views.remove(&id);
        let changes = Changes {
            removed_paths: BTreeSet::from([path]),
            ..Changes::default()
        };
        self.commit(&mut st, backup, changes, None, Vec::new(), Some(&me))
            .await?;
        Ok(Done { ok: true })
    }

    /// The view, if `me` owns it. A view the caller cannot see is "not
    /// found", so IDs of private views do not leak.
    fn own_view<'s>(
        st: &'s State,
        id: &ViewId,
        me: &UserRef,
    ) -> Result<&'s View, Error> {
        let not_found =
            || Error::new(ErrorKind::NotFound, format!("view {id}"));
        let view = st.views.get(id).ok_or_else(not_found)?;
        let member_of =
            |key: &TeamKey| st.graph.team(key).is_some_and(|t| t.is_member(me));
        if !view.visible_to(me, member_of) {
            return Err(not_found());
        }
        if &view.owner != me {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "only the view's owner can change it".to_owned(),
            ));
        }
        Ok(view)
    }

    pub(crate) async fn list_projects(
        &self,
        _rc: RequestContext,
        _: NoInput,
    ) -> Result<routes::ProjectsOut, Error> {
        let st = self.read_state().await;
        Ok(routes::ProjectsOut {
            projects: st
                .graph
                .projects()
                .map(|p| planning::project_out(&st, p, self.clock.now().date()))
                .collect(),
        })
    }

    pub(crate) async fn get_project(
        &self,
        _rc: RequestContext,
        i: ProjectRef,
    ) -> Result<ProjectOut, Error> {
        let st = self.read_state().await;
        let id = parse_id::<ProjectId>("project_id", &i.project_id)?;
        let p = st.graph.project(&id).ok_or_else(|| {
            Error::new(ErrorKind::NotFound, format!("unknown project {id}"))
        })?;
        Ok(planning::project_out(&st, p, self.clock.now().date()))
    }

    pub(crate) async fn list_comments(
        &self,
        _rc: RequestContext,
        i: ListComments,
    ) -> Result<ThreadsOut, Error> {
        let st = self.read_state().await;
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
        let st = self.read_state().await;
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
        let mut st = self.write_state().await?;
        let team = Self::team_key(&st.graph, &i.team)?;
        let mut draft = NewIssue::new(IssueId::generate(), team, &i.title);
        draft.body = i.description;
        draft.template = i.template;
        draft.state = i.state.as_deref().map(StatusKey::parse).transpose()?;
        draft.priority = i.priority.map(priority).transpose()?;
        draft.estimate = i.estimate.as_deref().map(estimate).transpose()?;
        draft.assignee =
            i.assignee.as_deref().map(UserRef::parse).transpose()?;
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
        draft.milestone = i
            .milestone
            .as_deref()
            .map(|m| parse_id("milestone", m))
            .transpose()?;
        draft.due = i.due.as_deref().map(date).transpose()?;
        draft.intake = i.intake;
        let repeat = match i.repeat.as_deref() {
            Some(text) => Some(text.parse::<Repeat>()?),
            None => draft.template.as_ref().and_then(|name| {
                st.graph
                    .team(&draft.team)
                    .and_then(|t| t.templates.iter().find(|x| &x.name == name))
                    .and_then(|x| x.repeat)
            }),
        };
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
        let mut changes = match st.graph.create_issue(draft, &ctx) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        if repeat.is_some() {
            match st.graph.set_repeat(&id, repeat, ctx.now) {
                Ok(more) => changes.issues.extend(more.issues),
                Err(e) => {
                    *st = backup;
                    return Err(e.into());
                }
            }
        }
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        let issue = st.graph.issue(&id).ok_or_else(|| {
            Error::new(ErrorKind::Internal, "issue vanished".to_owned())
        })?;
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
        let mut st = self.write_state().await?;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let current = st
            .graph
            .issue(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, i.key.clone()))?;
        let repeat = i
            .repeat
            .as_deref()
            .map(|t| (!t.is_empty()).then(|| t.parse::<Repeat>()).transpose())
            .transpose()?;
        let patch = patch_from(&st.graph, current, i, me)?;
        let backup = st.clone();
        let mut changes = match st.graph.update_issue(&id, patch, &ctx) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        if let Some(repeat) = repeat {
            match st.graph.set_repeat(&id, repeat, ctx.now) {
                Ok(more) => changes.issues.extend(more.issues),
                Err(e) => {
                    *st = backup;
                    return Err(e.into());
                }
            }
        }
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn move_issue(
        &self,
        rc: RequestContext,
        i: MoveIssue,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let to = Self::team_key(&st.graph, &i.to)?;
        let backup = st.clone();
        let changes = st.graph.move_issue(&id, &to, &ctx)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn triage_issue(
        &self,
        rc: RequestContext,
        i: TriageIssue,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let action = match i.action.as_str() {
            "accept" => TriageAction::Accept {
                status: i.state.as_deref().map(StatusKey::parse).transpose()?,
            },
            "decline" => TriageAction::Decline { reason: i.reason },
            "duplicate" => TriageAction::Duplicate {
                of: Self::issue_id(
                    &st.graph,
                    i.duplicate_of.as_deref().ok_or_else(|| {
                        invalid("duplicate needs duplicate_of")
                    })?,
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
            other => {
                return Err(invalid(&format!(
                    "unknown triage action {other:?}"
                )));
            }
        };
        let backup = st.clone();
        let changes = st.graph.triage(&id, &action, &ctx)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    fn issue_out(
        &self,
        st: &State,
        id: &IssueId,
        now: OffsetDateTime,
    ) -> Result<IssueOut, Error> {
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
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let mut team =
            Team::new(TeamId::generate(), TeamKey::parse(&i.key)?, &i.name)?;
        team.lead = i.lead.as_deref().map(UserRef::parse).transpose()?;
        team.members = i
            .members
            .iter()
            .map(|m| UserRef::parse(m))
            .collect::<Result<_, _>>()?;
        let backup = st.clone();
        let changes = match add_team(
            &mut st.graph,
            team.clone(),
            self.clock.now().date(),
        ) {
            Ok(c) => c,
            Err(e) => {
                *st = backup;
                return Err(e.into());
            }
        };
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        Ok(TeamOut::of(&team))
    }

    pub(crate) async fn put_team(
        &self,
        rc: RequestContext,
        i: PutTeam,
    ) -> Result<TeamOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let key = Self::team_key(&st.graph, &i.team)?;
        let path = format!("teams/{key}/team.yaml");
        let team = Team::from_yaml(&path, &i.yaml)?;
        if team.key != key {
            return Err(invalid(
                "the file's key must match the team in the path; move issues \
                    to rename a team",
            ));
        }
        let backup = st.clone();
        let changes = st.graph.update_team(team)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        st.graph
            .team(&key)
            .map(TeamOut::of)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, key.to_string()))
    }

    /// Archives or restores a team (D303).
    pub(crate) async fn set_team_archived(
        &self,
        rc: RequestContext,
        i: TeamRef,
        archived: bool,
    ) -> Result<TeamOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
        let key = Self::team_key(&st.graph, &i.team)?;
        let backup = st.clone();
        let changes = st.graph.set_team_archived(&key, archived)?;
        if !changes.teams.is_empty() {
            self.commit(
                &mut st,
                backup,
                changes,
                None,
                Vec::new(),
                ctx.actor.as_ref(),
            )
            .await?;
        }
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
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
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
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            vec![(path, text)],
            ctx.actor.as_ref(),
        )
        .await?;
        let p = st
            .graph
            .project(&id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, id.to_string()))?;
        Ok(planning::project_out(&st, p, ctx.today))
    }

    // ----- comments -----

    pub(crate) async fn add_comment(
        &self,
        rc: RequestContext,
        i: AddComment,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.write_state().await?;
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
            (None, Some(c)) => {
                st.comments.thread_of(&c).map(|t| t.target.clone())
            }
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
        self.commit(
            &mut st,
            backup,
            changes,
            Some(cc),
            Vec::new(),
            ctx.actor.as_ref(),
        )
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

    pub(crate) async fn react(
        &self,
        rc: RequestContext,
        i: React,
    ) -> Result<CommentResult, Error> {
        let ctx = self.ctx(&rc).await?;
        let id = parse_id::<CommentId>("comment_id", &i.comment_id)?;
        self.comment_command(ctx, &id, |c, ctx| {
            c.react(&id, &i.emoji, i.on, ctx)
        })
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
        let mut st = self.write_state().await?;
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
        self.commit(
            &mut st,
            backup,
            changes,
            Some(cc),
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        Ok(thread
            .map_or(CommentResult { thread: None }, |t| thread_result(&st, &t)))
    }
}

/// Adds a team and pre-creates its cycles.
fn add_team(
    graph: &mut WorkGraph,
    team: Team,
    today: Date,
) -> vellum_work::Result<Changes> {
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
        milestone: clearable(i.milestone)
            .map(|v| {
                v.as_deref()
                    .map(|m| parse_id::<MilestoneId>("milestone", m))
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
    Estimate::from_text(text).ok_or_else(|| {
        invalid(&format!("estimate {text:?} is not a number or size"))
    })
}

fn date(text: &str) -> Result<Date, Error> {
    Date::parse(text, format_description!("[year]-[month]-[day]"))
        .map_err(|_| invalid(&format!("date {text:?} must be YYYY-MM-DD")))
}

/// Saved views one person may own (RT-040): every view stays in memory
/// and is listed on each read.
const MAX_VIEWS_PER_PERSON: usize = 200;
/// The largest filter a saved view takes, as JSON text (RT-040).
const MAX_VIEW_FILTER_BYTES: usize = 16 * 1024;

/// An optional engine enum from its serde name.
fn pick<T: serde::de::DeserializeOwned>(
    field: &str,
    text: Option<&str>,
) -> Result<Option<T>, Error> {
    text.map(|t| parse_name(field, t)).transpose()
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
        StoreError::AlreadyExists(_)
        | StoreError::PreconditionFailed { .. } => ErrorKind::Conflict,
        StoreError::InvalidPath { .. } => ErrorKind::InvalidInput,
        _ => ErrorKind::Unavailable,
    };
    Error::new(kind, format!("Work storage: {e}")).with_source(e)
}

/// Lists the blob paths under `dir`; a missing directory is empty.
/// A name for this replica in journal entries: host, process and a
/// per-start nonce.
fn writer_name() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_default();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{host}/{}/{nonce:08x}", std::process::id())
}

/// Builds Work from every file in `store`.
async fn load_all(store: &dyn BlobStore) -> Result<(State, LoadReport), Error> {
    let mut state = State::default();
    let mut report = LoadReport::default();
    let mut paths = Vec::new();
    for dir in ["teams", "projects", "initiatives", ".comments", "views"] {
        paths.extend(list(store, dir).await?);
    }
    // Teams first: issues and cycles need their team.
    paths.sort_by_key(|p| (!p.ends_with("/team.yaml"), p.clone()));
    for path in paths {
        match load_one(store, &mut state, &path).await {
            Ok(true) => report.loaded += 1,
            Ok(false) => {}
            Err(e) => {
                warn!(path, error = %e, "skipped a Work file");
                report.skipped += 1;
            }
        }
    }
    Ok((state, report))
}

/// Brings `state` up to date with files another replica changed. Issues
/// and comment threads are reloaded one by one; a changed team, cycle or
/// project (rare) rebuilds everything, since they shape the rest.
///
/// # Errors
///
/// Whatever stops a file from being read; the caller then rebuilds.
async fn apply_paths(
    store: &dyn BlobStore,
    state: &mut State,
    paths: &[String],
) -> Result<(), Error> {
    for path in paths {
        match classify(path) {
            Kind::Issue => {
                let old = state
                    .graph
                    .issues()
                    .find(|i| i.path() == *path)
                    .map(|i| i.id);
                if let Some(id) = old {
                    let _ = state.graph.unload_issue(&id);
                }
                if let Some(text) = read_text(store, path).await? {
                    let issue = Issue::from_markdown(path, &text)?;
                    // The same issue may have moved here from another path.
                    let _ = state.graph.unload_issue(&issue.id);
                    state.graph.load_issue(issue)?;
                }
            }
            Kind::Thread => {
                let _ = state.comments.unload_path(path);
                if let Some(text) = read_text(store, path).await? {
                    let thread = Thread::from_yaml(path, &text)?;
                    let _ = state.comments.unload_path(&thread.path());
                    state.comments.load(thread)?;
                }
            }
            Kind::View => {
                state.views.retain(|_, v| v.path() != *path);
                if let Some(text) = read_text(store, path).await? {
                    let view = View::from_yaml(path, &text)?;
                    let _ = state.views.insert(view.id, view);
                }
            }
            Kind::Team
            | Kind::Cycle
            | Kind::Project
            | Kind::Update
            | Kind::Initiative => {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "team settings changed",
                ));
            }
            Kind::Other => {}
        }
    }
    Ok(())
}

/// A file's text, or `None` when it no longer exists.
async fn read_text(
    store: &dyn BlobStore,
    path: &str,
) -> Result<Option<String>, Error> {
    match store.get(&blob_path(path)?).await {
        Ok(blob) => String::from_utf8(blob.bytes.to_vec())
            .map(Some)
            .map_err(|_| invalid("not UTF-8")),
        Err(StoreError::NotFound(_)) => Ok(None),
        Err(e) => Err(store_error(e)),
    }
}

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
async fn load_one(
    store: &dyn BlobStore,
    state: &mut State,
    path: &str,
) -> Result<bool, Error> {
    let kind = classify(path);
    if kind == Kind::Other {
        return Ok(false);
    }
    let blob = store.get(&blob_path(path)?).await.map_err(store_error)?;
    let text =
        std::str::from_utf8(&blob.bytes).map_err(|_| invalid("not UTF-8"))?;
    match kind {
        Kind::Team => state.graph.insert_team(Team::from_yaml(path, text)?)?,
        Kind::Issue => {
            state.graph.load_issue(Issue::from_markdown(path, text)?)?
        }
        Kind::Cycle => state.graph.load_cycle(Cycle::from_yaml(path, text)?)?,
        Kind::Project => {
            let p = Project::from_markdown(path, text)?;
            let _ = state.project_paths.insert(p.id, path.to_owned());
            let _ = state.graph.upsert_project(p)?;
        }
        Kind::Thread => state.comments.load(Thread::from_yaml(path, text)?)?,
        Kind::View => {
            let view = View::from_yaml(path, text)?;
            let _ = state.views.insert(view.id, view);
        }
        Kind::Update | Kind::Initiative => {
            state.planning.load(kind == Kind::Update, path, text)?
        }
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
    View,
    Update,
    Initiative,
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
        ["projects", _, "updates", f] if has_ext(f, "md") => Kind::Update,
        ["initiatives", f] if has_ext(f, "md") => Kind::Initiative,
        [".comments", _, f] if has_ext(f, "yaml") => Kind::Thread,
        ["views", f] if has_ext(f, "yaml") => Kind::View,
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
            Kind::Update
        );
        assert_eq!(classify("initiatives/q4-launch.md"), Kind::Initiative);
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
