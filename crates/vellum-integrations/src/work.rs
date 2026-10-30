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

pub mod steps;

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tracing::{Instrument as _, info_span};
use vellum_work::{
    Changes, CommentChanges, Comments, Ctx, TeamKey, UserRef, WorkGraph,
};

use self::steps::{to_date, to_offset};
use crate::model::{
    IssueDraft, IssueKey, IssueSnapshot, WorkCommands, WorkError, WorkflowState,
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
    fn commit(
        &self,
        commit: Commit,
    ) -> impl Future<Output = Result<(), WorkError>> + Send;
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
        self.graph.lock().map_err(|_| {
            WorkError::Unavailable("work graph lock poisoned".to_owned())
        })
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
        self.comments.lock().map_err(|_| {
            WorkError::Unavailable("comment store lock poisoned".to_owned())
        })
    }

    /// The command context: now, today in the team's time zone, and the
    /// acting user.
    fn ctx(
        &self,
        graph: &WorkGraph,
        team: &TeamKey,
        actor: UserRef,
    ) -> Result<Ctx, WorkError> {
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
    async fn issue(
        &self,
        key: &IssueKey,
    ) -> Result<Option<IssueSnapshot>, WorkError> {
        self.with_graph(|graph| steps::snapshot(graph, key))
    }

    async fn states(
        &self,
        team: &str,
    ) -> Result<Vec<WorkflowState>, WorkError> {
        self.with_graph(|graph| steps::states(graph, team))
    }

    async fn set_state(
        &self,
        key: &IssueKey,
        state_id: &str,
        reason: &str,
    ) -> Result<(), WorkError> {
        let span = info_span!(
            "work.set_state",
            issue = %key,
            state = state_id,
            reason
        );
        async {
            let changes = self.with_graph(|graph| {
                let team = steps::team_of(graph, key)?;
                let ctx = self.ctx(graph, &team, self.bot.clone())?;
                steps::set_state(graph, key, state_id, &ctx)
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

    async fn create_issue(
        &self,
        draft: IssueDraft,
    ) -> Result<IssueKey, WorkError> {
        let span = info_span!(
            "work.create_issue",
            team = %draft.team,
            issue = tracing::field::Empty
        );
        async {
            let team = steps::team_key(&draft.team)?;
            let (key, changes) = self.with_graph(|graph| {
                let ctx = self.ctx(graph, &team, self.bot.clone())?;
                steps::create_issue(graph, draft, &ctx)
            })?;
            let _ = tracing::Span::current()
                .record("issue", tracing::field::display(&key));
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
            let actor = steps::actor(author, &self.bot)?;
            let (comments, changes) = self.with_graph(|graph| {
                let team = steps::team_of(graph, key)?;
                let ctx = self.ctx(graph, &team, actor)?;
                steps::add_comment(
                    graph,
                    &mut *self.comment_store()?,
                    key,
                    body,
                    &ctx,
                )
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
