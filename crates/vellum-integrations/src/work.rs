// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The [`WorkCommands`] port implemented over the issue tracker engine
//! (`vellum-work`).
//!
//! The engine is an in-memory model: a command changes the
//! [`WorkGraph`] and returns the files to write and the events to publish.
//! [`GraphWork`] runs each command as the integration's bot user, then
//! hands the result to a [`CommitSink`], which persists it and publishes
//! the events. The engine lock is never held across that `await`.
//!
//! Intake is idempotent: a draft from Sentry, email, Slack or an import
//! carries its source as an issue link, and a second draft with the same
//! source returns the existing issue instead of filing it twice.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tracing::{Instrument as _, info_span};
use vellum_work::issue::IssueLink;
use vellum_work::{
    Category, Changes, CommentChanges, Comments, Ctx, Estimate, IssueId, NewComment, NewIssue,
    StatusKey, Target, TeamKey, UserRef, WorkGraph,
};

use crate::model::{
    IssueDraft, IssueKey, IssueSnapshot, Priority, Source, StateCategory, WorkCommands, WorkError,
    WorkflowState,
};

/// What one command changed, for the sink to persist and publish.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    /// Issue, team and cycle files to write, and events to publish.
    pub changes: Changes,
    /// Comment thread files to write, and their events.
    pub comments: Option<CommentChanges>,
    /// Why the change was made, for the audit log ("GitHub
    /// `northstar/api#12` merged").
    pub reason: Option<String>,
}

/// Persists a [`Commit`] and publishes its events.
///
/// The engine has already applied the change in memory when the sink is
/// called. If the sink fails, the caller should reload the workspace so
/// memory matches what is on disk.
pub trait CommitSink: Send + Sync {
    /// Writes and publishes one commit.
    fn commit(&self, commit: Commit) -> impl Future<Output = Result<(), WorkError>> + Send;
}

/// [`WorkCommands`] over a shared [`WorkGraph`] and comment store.
#[derive(Debug)]
pub struct GraphWork<S> {
    graph: Arc<Mutex<WorkGraph>>,
    comments: Arc<Mutex<Comments>>,
    sink: S,
    bot: UserRef,
    clock: fn() -> Timestamp,
}

impl<S: CommitSink> GraphWork<S> {
    /// Runs commands as `bot`, the integration's user.
    pub fn new(
        graph: Arc<Mutex<WorkGraph>>,
        comments: Arc<Mutex<Comments>>,
        sink: S,
        bot: UserRef,
    ) -> Self {
        Self {
            graph,
            comments,
            sink,
            bot,
            clock: Timestamp::now,
        }
    }

    /// Uses `clock` instead of the system time (for tests).
    #[must_use]
    pub const fn with_clock(mut self, clock: fn() -> Timestamp) -> Self {
        self.clock = clock;
        self
    }

    fn graph(&self) -> Result<MutexGuard<'_, WorkGraph>, WorkError> {
        self.graph
            .lock()
            .map_err(|_| WorkError::Unavailable("work graph lock poisoned".to_owned()))
    }

    /// Runs `f` with the engine locked; the lock is released before any
    /// `await`.
    fn with_graph<T>(
        &self,
        f: impl FnOnce(&mut WorkGraph) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        f(&mut *self.graph()?)
    }

    fn comment_store(&self) -> Result<MutexGuard<'_, Comments>, WorkError> {
        self.comments
            .lock()
            .map_err(|_| WorkError::Unavailable("comment store lock poisoned".to_owned()))
    }

    /// The command context: now, today in the team's time zone, and the
    /// acting user.
    fn ctx(&self, graph: &WorkGraph, team: &TeamKey, actor: UserRef) -> Result<Ctx, WorkError> {
        let now = (self.clock)();
        let tz = graph
            .team(team)
            .and_then(|t| TimeZone::get(&t.timezone).ok())
            .unwrap_or(TimeZone::UTC);
        Ok(Ctx {
            now: to_offset(now)?,
            today: to_date(now.to_zoned(tz).date())?,
            actor: Some(actor),
        })
    }

    async fn commit(&self, commit: Commit) -> Result<(), WorkError> {
        if commit.changes == Changes::default() && commit.comments.is_none() {
            return Ok(());
        }
        self.sink.commit(commit).await
    }
}

impl<S: CommitSink> WorkCommands for GraphWork<S> {
    async fn issue(&self, key: &IssueKey) -> Result<Option<IssueSnapshot>, WorkError> {
        self.with_graph(|graph| {
            let Some(issue) = graph.resolve(&key.to_string()) else {
                return Ok(None);
            };
            let team = graph
                .team(&issue.team)
                .ok_or_else(|| WorkError::NotFound(issue.team.to_string()))?;
            let status = team
                .workflow
                .get(&issue.state)
                .ok_or_else(|| WorkError::NotFound(issue.state.to_string()))?;
            Ok(Some(IssueSnapshot {
                key: our_key(&issue.team, issue.number),
                title: issue.title.clone(),
                state: state(status),
                assignee: issue.assignee.as_ref().map(ToString::to_string),
                created_at: from_offset(issue.created_at)?,
            }))
        })
    }

    async fn states(&self, team: &str) -> Result<Vec<WorkflowState>, WorkError> {
        let key = team_key(team)?;
        self.with_graph(|graph| {
            let team = graph
                .team(&key)
                .ok_or_else(|| WorkError::NotFound(key.to_string()))?;
            Ok(team.workflow.statuses.iter().map(state).collect())
        })
    }

    async fn set_state(
        &self,
        key: &IssueKey,
        state_id: &str,
        reason: &str,
    ) -> Result<(), WorkError> {
        let span = info_span!("work.set_state", issue = %key, state = state_id, reason);
        async {
            let status = StatusKey::parse(state_id).map_err(refused)?;
            let changes = self.with_graph(|graph| {
                let (id, team) = find(graph, key)?;
                let ctx = self.ctx(graph, &team, self.bot.clone())?;
                graph.set_state(&id, status, &ctx).map_err(engine)
            })?;
            self.commit(Commit {
                changes,
                comments: None,
                reason: Some(reason.to_owned()),
            })
            .await
        }
        .instrument(span)
        .await
    }

    async fn create_issue(&self, draft: IssueDraft) -> Result<IssueKey, WorkError> {
        let span =
            info_span!("work.create_issue", team = %draft.team, issue = tracing::field::Empty);
        async {
            let team = team_key(&draft.team)?;
            let source = source_link(&draft.source);
            let (key, changes) = self.with_graph(|graph| {
                if let Some(link) = &source
                    && let Some(existing) = graph
                        .issues()
                        .find(|i| i.links.iter().any(|l| l.url == link.url))
                {
                    let key = our_key(&existing.team, existing.number);
                    tracing::info!(issue = %key, source = %link.url, "intake already filed");
                    return Ok((key, None));
                }
                let id = IssueId::generate();
                let new = new_issue(graph, id, team.clone(), draft, source)?;
                let ctx = self.ctx(graph, &team, self.bot.clone())?;
                let changes = graph.create_issue(new, &ctx).map_err(engine)?;
                let issue = graph
                    .issue(&id)
                    .ok_or_else(|| WorkError::Unavailable("created issue vanished".to_owned()))?;
                Ok((our_key(&issue.team, issue.number), Some(changes)))
            })?;
            let _ = tracing::Span::current().record("issue", tracing::field::display(&key));
            if let Some(changes) = changes {
                self.commit(Commit {
                    changes,
                    comments: None,
                    reason: None,
                })
                .await?;
            }
            Ok(key)
        }
        .instrument(span)
        .await
    }

    async fn add_comment(
        &self,
        key: &IssueKey,
        author: Option<&str>,
        body: &str,
    ) -> Result<(), WorkError> {
        let span = info_span!("work.add_comment", issue = %key);
        async {
            let actor = match author {
                Some(handle) => UserRef::parse(handle).map_err(refused)?,
                None => self.bot.clone(),
            };
            let (comments, changes) = self.with_graph(|graph| {
                let (id, team) = find(graph, key)?;
                let ctx = self.ctx(graph, &team, actor)?;
                let audience = graph
                    .issue(&id)
                    .map(|i| i.subscribers.clone())
                    .unwrap_or_default();
                let draft = NewComment::new(
                    vellum_kernel::CommentId::generate(),
                    Target::Issue { issue: id },
                    body,
                );
                let comments = self
                    .comment_store()?
                    .add(draft, &audience, &ctx)
                    .map_err(engine)?;
                let changes = graph.apply_comment(&comments, &ctx).map_err(engine)?;
                Ok((comments, changes))
            })?;
            self.commit(Commit {
                changes,
                comments: Some(comments),
                reason: None,
            })
            .await
        }
        .instrument(span)
        .await
    }
}

/// Builds the engine's draft from ours.
fn new_issue(
    graph: &WorkGraph,
    id: IssueId,
    team: TeamKey,
    draft: IssueDraft,
    source: Option<IssueLink>,
) -> Result<NewIssue, WorkError> {
    let intake = matches!(
        draft.source,
        Source::Slack { .. } | Source::Sentry { .. } | Source::Email { .. } | Source::Form { .. }
    );
    let mut new = NewIssue::new(id, team, &draft.title);
    new.body = draft.description;
    new.intake = intake;
    new.priority = draft.priority.map(priority);
    // Outside sources name labels the team may not have (Sentry adds
    // `sentry`); filing the issue matters more, so unknown ones are dropped.
    let known = |label: &str| {
        graph
            .team(&new.team)
            .into_iter()
            .flat_map(|t| t.labels.iter())
            .chain(graph.workspace_labels())
            .any(|l| l.reference().eq_ignore_ascii_case(label))
    };
    let labels: Vec<String> = draft.labels.into_iter().filter(|l| known(l)).collect();
    new.labels = labels;
    new.assignee = draft
        .assignee
        .as_deref()
        .map(UserRef::parse)
        .transpose()
        .map_err(refused)?;
    new.estimate = draft.estimate.as_deref().and_then(Estimate::from_text);
    new.project = draft.project.as_deref().and_then(|p| {
        graph
            .projects()
            .find(|project| project.id.to_string() == p || project.title.eq_ignore_ascii_case(p))
            .map(|project| project.id)
    });
    new.due = draft.due.map(to_date).transpose()?;
    new.state = draft
        .state
        .as_deref()
        .map(StatusKey::parse)
        .transpose()
        .map_err(refused)?;
    new.parent = draft
        .parent
        .map(|p| find(graph, &p).map(|(parent, _)| parent))
        .transpose()?;
    new.links.extend(source);
    Ok(new)
}

/// The link that identifies where an issue came from, used to avoid
/// filing the same report twice.
fn source_link(source: &Source) -> Option<IssueLink> {
    let (url, kind, title) = match source {
        Source::Slack {
            permalink: Some(url),
        } => (url.clone(), "slack", "Slack message"),
        Source::Sentry { issue_id } => {
            (format!("sentry:issue/{issue_id}"), "sentry", "Sentry issue")
        }
        Source::Email { message_id, .. } => (
            format!("mid:{}", message_id.trim_matches(['<', '>'])),
            "email",
            "Email",
        ),
        Source::Import {
            tracker,
            external_id,
        } => (
            format!("import:{tracker}/{external_id}"),
            "import",
            "Imported issue",
        ),
        Source::App
        | Source::Slack { permalink: None }
        | Source::Form { .. }
        | Source::Recurring { .. } => {
            return None;
        }
    };
    Some(IssueLink {
        url,
        title: Some(title.to_owned()),
        kind: Some(kind.to_owned()),
    })
}

fn find(graph: &WorkGraph, key: &IssueKey) -> Result<(IssueId, TeamKey), WorkError> {
    graph
        .resolve(&key.to_string())
        .map(|i| (i.id, i.team.clone()))
        .ok_or_else(|| WorkError::NotFound(key.to_string()))
}

fn team_key(team: &str) -> Result<TeamKey, WorkError> {
    TeamKey::parse(team).map_err(|_| WorkError::NotFound(team.to_owned()))
}

fn our_key(team: &TeamKey, number: u64) -> IssueKey {
    IssueKey {
        team: team.to_string(),
        number,
    }
}

fn state(status: &vellum_work::Status) -> WorkflowState {
    WorkflowState {
        id: status.key.to_string(),
        name: status.name.clone(),
        category: match status.category {
            Category::Triage => StateCategory::Triage,
            Category::Backlog => StateCategory::Backlog,
            Category::Unstarted => StateCategory::Unstarted,
            Category::Started => StateCategory::Started,
            Category::Completed => StateCategory::Completed,
            Category::Canceled => StateCategory::Canceled,
            Category::Duplicate => StateCategory::Duplicate,
        },
    }
}

const fn priority(p: Priority) -> vellum_work::Priority {
    match p {
        Priority::None => vellum_work::Priority::None,
        Priority::Urgent => vellum_work::Priority::Urgent,
        Priority::High => vellum_work::Priority::High,
        Priority::Normal => vellum_work::Priority::Normal,
        Priority::Low => vellum_work::Priority::Low,
    }
}

fn engine(error: vellum_work::WorkError) -> WorkError {
    match error {
        vellum_work::WorkError::UnknownIssue(what) => WorkError::NotFound(what),
        vellum_work::WorkError::UnknownTeam(team) => WorkError::NotFound(team.to_string()),
        other => WorkError::Refused(other.to_string()),
    }
}

fn refused(error: impl std::fmt::Display) -> WorkError {
    WorkError::Refused(error.to_string())
}

fn to_offset(ts: Timestamp) -> Result<time::OffsetDateTime, WorkError> {
    time::OffsetDateTime::from_unix_timestamp_nanos(ts.as_nanosecond())
        .map_err(|e| WorkError::Refused(format!("timestamp out of range: {e}")))
}

fn from_offset(at: time::OffsetDateTime) -> Result<Timestamp, WorkError> {
    Timestamp::from_nanosecond(at.unix_timestamp_nanos())
        .map_err(|e| WorkError::Refused(format!("timestamp out of range: {e}")))
}

fn to_date(date: jiff::civil::Date) -> Result<time::Date, WorkError> {
    let bad = |e: time::error::ComponentRange| WorkError::Refused(format!("bad date: {e}"));
    let month = u8::try_from(date.month())
        .map_err(|_| WorkError::Refused("bad month".to_owned()))
        .and_then(|m| time::Month::try_from(m).map_err(bad))?;
    let day = u8::try_from(date.day()).map_err(|_| WorkError::Refused("bad day".to_owned()))?;
    time::Date::from_calendar_date(i32::from(date.year()), month, day).map_err(bad)
}
