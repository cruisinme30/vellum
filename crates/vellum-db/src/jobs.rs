// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A durable job queue in the index database (#63, ADR 0038 D162).
//!
//! Any replica enqueues; any replica claims. On Postgres a claim takes the
//! oldest due jobs with `FOR UPDATE SKIP LOCKED`, so replicas never wait
//! on each other or take the same job; on SQLite one claiming statement
//! does the same, since SQLite has one writer. A claimed job carries a
//! lease: a replica that dies mid-job loses it when the lease runs out,
//! and another replica runs the job again. Handlers must therefore be
//! idempotent. An enqueue with an idempotency key that is still ready or
//! running returns the existing job instead of adding a second one.
//!
//! Failures retry with exponential backoff up to the job's attempt limit,
//! then the job stays `failed` for an operator to inspect. Each queue is
//! bounded: enqueueing past the limit fails with [`Error::QueueFull`]
//! rather than letting a stuck consumer grow the table without end.
//!
//! [`Worker`] runs a [`Handler`] over one queue: it claims, extends the
//! lease while a job runs, records the outcome, and wraps each attempt in
//! a `job` span linked to the trace that enqueued it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::{JoinHandle, JoinSet};
use tracing::Instrument as _;

use crate::cluster::now_ms;
use crate::error::{Error, Result};
use crate::index::{Index, IndexPool};
use crate::telemetry::traced;

/// Where a job is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Waiting for its `run_at` time and a free worker.
    Ready,
    /// Claimed by a worker until its lease runs out.
    Running,
    /// Finished.
    Done,
    /// Gave up after its last attempt, or failed for good.
    Failed,
}

impl State {
    fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => Self::Ready,
        }
    }
}

/// A job to enqueue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// The queue, such as `"notify"`; a worker claims from one queue.
    pub queue: String,
    /// What the handler should do, such as `"webhook.deliver"`.
    pub kind: String,
    /// The handler's input, usually JSON.
    pub payload: String,
    /// When set, a second enqueue with the same key in the same queue,
    /// while the first is still ready or running, returns the first job.
    pub key: Option<String>,
    /// Earliest start, in milliseconds since the Unix epoch (0: now).
    pub run_at_ms: i64,
    /// Most attempts before the job is marked failed.
    pub max_attempts: u32,
    /// The W3C `traceparent` of the request that enqueued it.
    pub traceparent: Option<String>,
}

impl NewJob {
    /// A job to run now, at most five times.
    #[must_use]
    pub fn new(queue: &str, kind: &str, payload: impl Into<String>) -> Self {
        Self {
            queue: queue.to_owned(),
            kind: kind.to_owned(),
            payload: payload.into(),
            key: None,
            run_at_ms: 0,
            max_attempts: 5,
            traceparent: None,
        }
    }

    /// With an idempotency key.
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Not before `at_ms`.
    #[must_use]
    pub const fn at(mut self, at_ms: i64) -> Self {
        self.run_at_ms = at_ms;
        self
    }

    /// With an attempt limit (at least one).
    #[must_use]
    pub fn attempts(mut self, n: u32) -> Self {
        self.max_attempts = n.max(1);
        self
    }

    /// Linked to the trace that enqueued it.
    #[must_use]
    pub fn traced(mut self, traceparent: impl Into<String>) -> Self {
        self.traceparent = Some(traceparent.into());
        self
    }
}

/// A claimed job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Unique ID.
    pub id: i64,
    /// Its queue.
    pub queue: String,
    /// What to do.
    pub kind: String,
    /// The input.
    pub payload: String,
    /// This attempt's number, from 1.
    pub attempt: u32,
    /// Most attempts.
    pub max_attempts: u32,
    /// The enqueuing trace.
    pub traceparent: Option<String>,
}

/// What [`Queue::enqueue`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enqueued {
    /// The job's ID: the new one, or the one already holding the key.
    pub id: i64,
    /// The key was already queued, so nothing new was added.
    pub duplicate: bool,
}

/// How many jobs of one queue are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Waiting.
    pub ready: u64,
    /// Claimed.
    pub running: u64,
    /// Finished.
    pub done: u64,
    /// Gave up.
    pub failed: u64,
}

/// Runs `$body` on whichever pool the index uses, with `$system` set to
/// its `db.system`. The SQL uses `$n` placeholders, which both drivers
/// bind by number.
macro_rules! each {
    ($index:expr, $pool:ident, $system:ident => $body:expr) => {
        match $index.pool() {
            IndexPool::Sqlite($pool) => {
                let $system = "sqlite";
                $body
            }
            IndexPool::Postgres($pool) => {
                let $system = "postgresql";
                $body
            }
        }
    };
}

const ENQUEUE: &str = "INSERT INTO jobs (queue, kind, payload, idem_key, \
     state, attempts, max_attempts, run_at_ms, traceparent, created_ms, \
     updated_ms) VALUES ($1, $2, $3, $4, 'ready', 0, $5, $6, $7, $8, $8) ON \
     CONFLICT (queue, idem_key) WHERE idem_key IS NOT NULL AND state IN \
     ('ready', 'running') DO NOTHING RETURNING id";

/// Claims due jobs. `$lock` is empty on SQLite, which has one writer, so
/// the subquery and update are atomic; Postgres skips rows another claim
/// has locked.
macro_rules! claim_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE jobs SET state = 'running', attempts = attempts + 1, \
             worker = $2, lease_until_ms = $3, updated_ms = $4 WHERE id IN ( \
             SELECT id FROM jobs WHERE queue = $1 AND ((state = 'ready' AND \
             run_at_ms <= $4) OR (state = 'running' AND lease_until_ms < $4)) \
             ORDER BY run_at_ms, id LIMIT $5 ",
            $lock,
            ") RETURNING id, queue, kind, payload, attempts, max_attempts, \
                traceparent"
        )
    };
}

type Row = (i64, String, String, String, i64, i64, Option<String>);

fn job((id, queue, kind, payload, attempts, max, traceparent): Row) -> Job {
    Job {
        id,
        queue,
        kind,
        payload,
        attempt: u32::try_from(attempts).unwrap_or(u32::MAX),
        max_attempts: u32::try_from(max).unwrap_or(u32::MAX),
        traceparent,
    }
}

fn ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// The job table in an index (migrated by `Index::open`).
#[derive(Debug, Clone)]
pub struct Queue {
    index: Index,
    limit: u64,
}

impl Queue {
    /// Most unfinished jobs one queue holds by default.
    pub const LIMIT: u64 = 100_000;

    /// The queue table in `index`, bounded at [`Self::LIMIT`] per queue.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self {
            index,
            limit: Self::LIMIT,
        }
    }

    /// With a different bound per queue (at least one).
    #[must_use]
    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// Adds `new` to its queue.
    ///
    /// # Errors
    /// [`Error::QueueFull`] when the queue holds its limit of unfinished
    /// jobs, or the database is unreachable.
    pub async fn enqueue(&self, new: &NewJob) -> Result<Enqueued> {
        self.enqueue_at(new, now_ms()).await
    }

    /// [`Self::enqueue`] at a given time, for tests.
    ///
    /// # Errors
    /// As [`Self::enqueue`].
    pub async fn enqueue_at(&self, new: &NewJob, now: i64) -> Result<Enqueued> {
        if let Some(key) = &new.key
            && let Some(id) = self.open_by_key(&new.queue, key).await?
        {
            return Ok(Enqueued {
                id,
                duplicate: true,
            });
        }
        // The bound is checked before the insert, not with it, so several
        // replicas enqueueing at once can pass it by a few jobs.
        let open = self.counts(&new.queue).await?;
        if open.ready + open.running >= self.limit {
            return Err(Error::QueueFull {
                queue: new.queue.clone(),
                limit: self.limit,
            });
        }
        let run_at = if new.run_at_ms > 0 {
            new.run_at_ms
        } else {
            now
        };
        let inserted: Option<i64> = each!(self.index, pool, system => traced(
            system,
            "jobs.enqueue",
            sqlx::query_scalar(ENQUEUE)
                .bind(&new.queue)
                .bind(&new.kind)
                .bind(&new.payload)
                .bind(&new.key)
                .bind(i64::from(new.max_attempts.max(1)))
                .bind(run_at)
                .bind(&new.traceparent)
                .bind(now)
                .fetch_optional(pool),
        )
        .await?);
        match (inserted, &new.key) {
            (Some(id), _) => Ok(Enqueued {
                id,
                duplicate: false,
            }),
            // Another replica enqueued the same key meanwhile.
            (None, Some(key)) => {
                let id = self
                    .open_by_key(&new.queue, key)
                    .await?
                    .unwrap_or_default();
                Ok(Enqueued {
                    id,
                    duplicate: true,
                })
            }
            (None, None) => Err(Error::Config(
                "a job without a key was not inserted".to_owned(),
            )),
        }
    }

    async fn open_by_key(&self, queue: &str, key: &str) -> Result<Option<i64>> {
        Ok(each!(self.index, pool, system => traced(
            system,
            "jobs.by_key",
            sqlx::query_scalar(
                "SELECT id FROM jobs WHERE queue = $1 AND idem_key = $2 \
                 AND state IN ('ready', 'running')",
            )
            .bind(queue)
            .bind(key)
            .fetch_optional(pool),
        )
        .await?))
    }

    /// Claims up to `n` due jobs of `queue` for `worker`, each until
    /// `lease` from now. A job whose last holder's lease ran out is due
    /// again; one that has used its last attempt that way is failed.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn claim(
        &self,
        queue: &str,
        worker: &str,
        n: usize,
        lease: Duration,
    ) -> Result<Vec<Job>> {
        self.claim_at(queue, worker, n, lease, now_ms()).await
    }

    /// [`Self::claim`] at a given time, for tests.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn claim_at(
        &self,
        queue: &str,
        worker: &str,
        n: usize,
        lease: Duration,
        now: i64,
    ) -> Result<Vec<Job>> {
        let n = i64::try_from(n.max(1)).unwrap_or(i64::MAX);
        let until = now.saturating_add(ms(lease));
        let _ = each!(self.index, pool, system => traced(
            system,
            "jobs.expire",
            sqlx::query(
                "UPDATE jobs SET state = 'failed', worker = NULL, \
                 lease_until_ms = NULL, last_error = 'the worker stopped \
                 during the last attempt', updated_ms = $2 WHERE queue = $1 \
                 AND state = 'running' AND lease_until_ms < $2 AND attempts >= \
                 max_attempts",
            )
            .bind(queue)
            .bind(now)
            .execute(pool),
        )
        .await?
        .rows_affected());
        let rows: Vec<Row> = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    "sqlite",
                    "jobs.claim",
                    sqlx::query_as(claim_sql!(""))
                        .bind(queue)
                        .bind(worker)
                        .bind(until)
                        .bind(now)
                        .bind(n)
                        .fetch_all(pool),
                )
                .await?
            }
            IndexPool::Postgres(pool) => {
                traced(
                    "postgresql",
                    "jobs.claim",
                    sqlx::query_as(claim_sql!("FOR UPDATE SKIP LOCKED"))
                        .bind(queue)
                        .bind(worker)
                        .bind(until)
                        .bind(now)
                        .bind(n)
                        .fetch_all(pool),
                )
                .await?
            }
        };
        let mut jobs: Vec<Job> = rows.into_iter().map(job).collect();
        jobs.sort_by_key(|j| j.id);
        Ok(jobs)
    }

    /// Marks `id` done. `false` when `worker` no longer holds it (its
    /// lease ran out and another worker took it).
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn complete(&self, id: i64, worker: &str) -> Result<bool> {
        let done = each!(self.index, pool, system => traced(
            system,
            "jobs.complete",
            sqlx::query(
                "UPDATE jobs SET state = 'done', worker = NULL, lease_until_ms \
                 = NULL, last_error = NULL, updated_ms = $3 WHERE id = $1 AND \
                 worker = $2 AND state = 'running'",
            )
            .bind(id)
            .bind(worker)
            .bind(now_ms())
            .execute(pool),
        )
        .await?
        .rows_affected());
        Ok(done == 1)
    }

    /// Records a failed attempt of `id`: the job runs again at
    /// `retry_at_ms` unless it used its last attempt, or `retry_at_ms` is
    /// `None` (the failure is permanent); then it is failed. `false` when
    /// `worker` no longer holds it.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn fail(
        &self,
        id: i64,
        worker: &str,
        error: &str,
        retry_at_ms: Option<i64>,
    ) -> Result<bool> {
        let now = now_ms();
        let give_up = retry_at_ms.is_none();
        let gone = each!(self.index, pool, system => traced(
            system,
            "jobs.fail",
            sqlx::query(
                "UPDATE jobs SET state = CASE WHEN $5 OR attempts >= \
                 max_attempts THEN 'failed' ELSE 'ready' END, run_at_ms = $3, \
                 worker = NULL, lease_until_ms = NULL, last_error = $4, \
                 updated_ms = $6 WHERE id = $1 AND worker = $2 AND state = \
                 'running'",
            )
            .bind(id)
            .bind(worker)
            .bind(retry_at_ms.unwrap_or(now))
            .bind(error)
            .bind(give_up)
            .bind(now)
            .execute(pool),
        )
        .await?
        .rows_affected());
        Ok(gone == 1)
    }

    /// Keeps `id` claimed by `worker` until `lease` from now. `false` when
    /// the worker lost it.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn extend(
        &self,
        id: i64,
        worker: &str,
        lease: Duration,
    ) -> Result<bool> {
        let until = now_ms().saturating_add(ms(lease));
        let kept = each!(self.index, pool, system => traced(
            system,
            "jobs.extend",
            sqlx::query(
                "UPDATE jobs SET lease_until_ms = $3 \
                 WHERE id = $1 AND worker = $2 AND state = 'running'",
            )
            .bind(id)
            .bind(worker)
            .bind(until)
            .execute(pool),
        )
        .await?
        .rows_affected());
        Ok(kept == 1)
    }

    /// Jobs of `queue` by state.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn counts(&self, queue: &str) -> Result<Counts> {
        let rows: Vec<(String, i64)> = each!(self.index, pool, system => traced(
            system,
            "jobs.counts",
            sqlx::query_as("SELECT state, COUNT(*) FROM jobs WHERE queue = $1 \
                GROUP BY state")
                .bind(queue)
                .fetch_all(pool),
        )
        .await?);
        let mut out = Counts::default();
        for (state, n) in rows {
            let n = u64::try_from(n).unwrap_or_default();
            match State::parse(&state) {
                State::Ready => out.ready = n,
                State::Running => out.running = n,
                State::Done => out.done = n,
                State::Failed => out.failed = n,
            }
        }
        Ok(out)
    }

    /// The state, attempts so far and last error of `id`, for operators
    /// and tests.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn status(
        &self,
        id: i64,
    ) -> Result<Option<(State, u32, Option<String>)>> {
        let row: Option<(String, i64, Option<String>)> = each!(self.index, pool, system => traced(
            system,
            "jobs.status",
            sqlx::query_as("SELECT state, attempts, last_error FROM jobs WHERE \
                id = $1")
                .bind(id)
                .fetch_optional(pool),
        )
        .await?);
        Ok(row.map(|(s, a, e)| {
            (State::parse(&s), u32::try_from(a).unwrap_or_default(), e)
        }))
    }

    /// Deletes jobs that finished or failed more than `older_than` ago.
    ///
    /// # Errors
    /// When the database is unreachable.
    pub async fn prune(&self, older_than: Duration) -> Result<u64> {
        let before = now_ms().saturating_sub(ms(older_than));
        Ok(each!(self.index, pool, system => traced(
            system,
            "jobs.prune",
            sqlx::query("DELETE FROM jobs WHERE state IN ('done', 'failed') \
                AND updated_ms <= $1")
                .bind(before)
                .execute(pool),
        )
        .await?
        .rows_affected()))
    }
}

/// How an attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Finished.
    Done,
    /// Failed, worth another attempt after the backoff.
    Retry(String),
    /// Failed for good; no more attempts.
    Fail(String),
}

/// A boxed future, as handlers return.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Does the work of one queue's jobs. Must be idempotent: a job whose
/// worker stops mid-attempt runs again.
pub trait Handler: Send + Sync + 'static {
    /// Runs one attempt of `job`.
    fn run<'a>(&'a self, job: &'a Job) -> BoxFuture<'a, Outcome>;
}

/// Worker settings.
#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    /// Most jobs run at once.
    pub concurrency: usize,
    /// Wait between claims when the queue is empty.
    pub poll: Duration,
    /// How long a claim lasts; extended every third of it while running.
    pub lease: Duration,
    /// Delay before the second attempt; doubles each attempt.
    pub backoff: Duration,
    /// Longest delay between attempts.
    pub max_backoff: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            poll: Duration::from_secs(1),
            lease: Duration::from_mins(1),
            backoff: Duration::from_secs(2),
            max_backoff: Duration::from_mins(10),
        }
    }
}

impl WorkerConfig {
    /// When attempt `attempt` (from 1) of job `id` failed at `now`, the
    /// time of the next one: the backoff doubled per attempt, capped,
    /// plus up to a second of jitter derived from the ID, so jobs that
    /// failed together do not retry together.
    #[must_use]
    pub fn retry_at(&self, id: i64, attempt: u32, now: i64) -> i64 {
        let doubled = self
            .backoff
            .saturating_mul(1_u32 << attempt.saturating_sub(1).min(20));
        let delay = ms(doubled.min(self.max_backoff));
        let jitter = id.wrapping_mul(7919).rem_euclid(1000);
        now.saturating_add(delay).saturating_add(jitter)
    }
}

/// Runs a [`Handler`] over one queue.
#[derive(Debug)]
pub struct Worker;

impl Worker {
    /// Claims and runs jobs of `queue_name` as `worker` until the handle
    /// is aborted.
    #[must_use]
    pub fn spawn(
        queue: Queue,
        queue_name: &str,
        worker: &str,
        handler: Arc<dyn Handler>,
        config: WorkerConfig,
    ) -> JoinHandle<()> {
        let (name, worker) = (queue_name.to_owned(), worker.to_owned());
        tokio::spawn(async move {
            let mut running = JoinSet::new();
            let slots = config.concurrency.max(1);
            loop {
                while running.try_join_next().is_some() {}
                let free = slots.saturating_sub(running.len());
                let claimed = if free == 0 {
                    Vec::new()
                } else {
                    match queue.claim(&name, &worker, free, config.lease).await
                    {
                        Ok(jobs) => jobs,
                        Err(error) => {
                            tracing::warn!(%error, queue = %name, "cannot \
                                claim jobs");
                            Vec::new()
                        }
                    }
                };
                if claimed.is_empty() {
                    // Wake when a job ends or the poll interval passes.
                    tokio::select! {
                        _ = running.join_next(), if !running.is_empty() => {}
                        () = tokio::time::sleep(config.poll) => {}
                    }
                    continue;
                }
                for job in claimed {
                    let span = tracing::info_span!(
                        "job",
                        job.queue = %job.queue,
                        job.kind = %job.kind,
                        job.id = job.id,
                        job.attempt = job.attempt,
                        job.traceparent = job.traceparent.as_deref().unwrap_or_default(),
                    );
                    let attempt = attempt(
                        queue.clone(),
                        worker.clone(),
                        Arc::clone(&handler),
                        config,
                        job,
                    );
                    let _ = running.spawn(attempt.instrument(span));
                }
            }
        })
    }
}

/// Runs one claimed job, extending its lease, and records the outcome.
async fn attempt(
    queue: Queue,
    worker: String,
    handler: Arc<dyn Handler>,
    config: WorkerConfig,
    job: Job,
) {
    let mut renew = tokio::time::interval(config.lease / 3);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = renew.tick().await;
    let run = handler.run(&job);
    tokio::pin!(run);
    let outcome = loop {
        tokio::select! {
            outcome = &mut run => break outcome,
            _ = renew.tick() => match queue.extend(
                job.id,
                &worker,
                config.lease
            ).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("lost the job's lease; another worker runs \
                        it");
                    return;
                }
                Err(error) => tracing::warn!(%error, "cannot extend the job's \
                    lease"),
            },
        }
    };
    let recorded = match &outcome {
        Outcome::Done => queue.complete(job.id, &worker).await,
        Outcome::Retry(error) => {
            let at = config.retry_at(job.id, job.attempt, now_ms());
            queue.fail(job.id, &worker, error, Some(at)).await
        }
        Outcome::Fail(error) => queue.fail(job.id, &worker, error, None).await,
    };
    match (outcome, recorded) {
        (Outcome::Done, Ok(true)) => tracing::debug!("job done"),
        (Outcome::Retry(error) | Outcome::Fail(error), Ok(true)) => {
            tracing::warn!(%error, "job attempt failed");
        }
        (_, Ok(false)) => {
            tracing::warn!("job outcome not recorded: the lease was lost")
        }
        (_, Err(error)) => tracing::warn!(%error, "job outcome not recorded"),
    }
}
