// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The job queue (ADR 0038 D162) on the SQLite backend.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use vellum_db::jobs::{
    BoxFuture, Handler, Job, NewJob, Outcome, Queue, State, Worker,
    WorkerConfig,
};
use vellum_db::{Error, Index, IndexConfig};

async fn queue(dir: &tempfile::TempDir) -> Queue {
    Queue::new(
        Index::open(&IndexConfig::sqlite(
            dir.path().join("index.sqlite"),
            None,
        ))
        .await
        .unwrap(),
    )
}

const LEASE: Duration = Duration::from_secs(10);

#[tokio::test]
async fn a_key_is_queued_once_while_open() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await;
    let job = NewJob::new("notify", "webhook.deliver", "{}").key("event-1");
    let first = q.enqueue_at(&job, 1_000).await.unwrap();
    assert!(!first.duplicate);
    let again = q.enqueue_at(&job, 1_001).await.unwrap();
    assert_eq!((again.id, again.duplicate), (first.id, true));

    // Still open while running.
    let claimed = q.claim_at("notify", "a", 5, LEASE, 2_000).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert!(q.enqueue_at(&job, 2_001).await.unwrap().duplicate);

    // Once done, the key can be queued again.
    assert!(q.complete(first.id, "a").await.unwrap());
    let later = q.enqueue_at(&job, 3_000).await.unwrap();
    assert!(!later.duplicate);
    assert_ne!(later.id, first.id);
}

#[tokio::test]
async fn a_full_queue_refuses_more() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await.with_limit(2);
    for i in 0..2 {
        let _ = q
            .enqueue(&NewJob::new("digest", "send", format!("{i}")))
            .await
            .unwrap();
    }
    let err = q
        .enqueue(&NewJob::new("digest", "send", "3"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::QueueFull { limit: 2, .. }));
    // Other queues are bounded separately.
    let _ = q.enqueue(&NewJob::new("other", "x", "")).await.unwrap();
}

#[tokio::test]
async fn claims_take_due_jobs_once_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await;
    let a = q
        .enqueue_at(&NewJob::new("q", "k", "a"), 1_000)
        .await
        .unwrap();
    let b = q
        .enqueue_at(&NewJob::new("q", "k", "b"), 1_000)
        .await
        .unwrap();
    let _later = q
        .enqueue_at(&NewJob::new("q", "k", "c").at(9_000), 1_000)
        .await
        .unwrap();

    let first = q.claim_at("q", "w1", 1, LEASE, 2_000).await.unwrap();
    assert_eq!(first.iter().map(|j| j.id).collect::<Vec<_>>(), [a.id]);
    assert_eq!(first[0].attempt, 1);
    let second = q.claim_at("q", "w2", 5, LEASE, 2_000).await.unwrap();
    // c is not due yet; a is taken.
    assert_eq!(second.iter().map(|j| j.id).collect::<Vec<_>>(), [b.id]);
    assert!(
        q.claim_at("q", "w3", 5, LEASE, 2_000)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_expired_lease_hands_the_job_on() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await;
    let id = q
        .enqueue_at(&NewJob::new("q", "k", "").attempts(2), 0)
        .await
        .unwrap()
        .id;
    let _ = q.claim_at("q", "dead", 1, LEASE, 1_000).await.unwrap();
    // Before the lease runs out nobody else gets it.
    assert!(
        q.claim_at("q", "b", 1, LEASE, 5_000)
            .await
            .unwrap()
            .is_empty()
    );
    let taken = q.claim_at("q", "b", 1, LEASE, 12_000).await.unwrap();
    assert_eq!(taken[0].attempt, 2);
    // The first worker can no longer record anything.
    assert!(!q.complete(id, "dead").await.unwrap());
    // b dies too, on the last attempt: the job fails.
    assert!(
        q.claim_at("q", "c", 1, LEASE, 30_000)
            .await
            .unwrap()
            .is_empty()
    );
    let (state, attempts, error) = q.status(id).await.unwrap().unwrap();
    assert_eq!((state, attempts), (State::Failed, 2));
    assert!(error.unwrap().contains("stopped"));
}

#[tokio::test]
async fn failures_retry_then_give_up() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await;
    let id = q
        .enqueue_at(&NewJob::new("q", "k", "").attempts(2), 0)
        .await
        .unwrap()
        .id;
    let _ = q.claim_at("q", "a", 1, LEASE, 0).await.unwrap();
    assert!(q.fail(id, "a", "503", Some(4_000)).await.unwrap());
    assert_eq!(q.status(id).await.unwrap().unwrap().0, State::Ready);
    assert!(
        q.claim_at("q", "a", 1, LEASE, 3_000)
            .await
            .unwrap()
            .is_empty()
    );
    let _ = q.claim_at("q", "a", 1, LEASE, 4_000).await.unwrap();
    assert!(q.fail(id, "a", "503 again", Some(8_000)).await.unwrap());
    let (state, _, error) = q.status(id).await.unwrap().unwrap();
    assert_eq!(state, State::Failed);
    assert_eq!(error.as_deref(), Some("503 again"));

    // A permanent failure stops at once.
    let other = q
        .enqueue_at(&NewJob::new("q", "k", ""), 0)
        .await
        .unwrap()
        .id;
    let _ = q.claim_at("q", "a", 1, LEASE, 9_000).await.unwrap();
    assert!(q.fail(other, "a", "410", None).await.unwrap());
    assert_eq!(q.status(other).await.unwrap().unwrap().0, State::Failed);

    let counts = q.counts("q").await.unwrap();
    assert_eq!((counts.failed, counts.ready), (2, 0));
    assert_eq!(q.prune(Duration::ZERO).await.unwrap(), 2);
}

#[test]
fn backoff_doubles_up_to_the_cap() {
    let config = WorkerConfig {
        backoff: Duration::from_secs(2),
        max_backoff: Duration::from_secs(10),
        ..WorkerConfig::default()
    };
    let delay = |attempt| config.retry_at(0, attempt, 0);
    assert_eq!(
        [delay(1), delay(2), delay(3), delay(4), delay(30)],
        [2_000, 4_000, 8_000, 10_000, 10_000]
    );
    assert!(config.retry_at(7, 1, 0) - 2_000 < 1_000);
}

/// Fails the first attempt of every job, then succeeds.
#[derive(Default)]
struct Flaky(AtomicU32);

impl Handler for Flaky {
    fn run<'a>(&'a self, job: &'a Job) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            let _ = self.0.fetch_add(1, Ordering::SeqCst);
            if job.attempt == 1 {
                Outcome::Retry("try again".to_owned())
            } else {
                Outcome::Done
            }
        })
    }
}

#[tokio::test]
async fn a_worker_runs_jobs_and_retries_failures() {
    let dir = tempfile::tempdir().unwrap();
    let q = queue(&dir).await;
    let ids = [
        q.enqueue(&NewJob::new("q", "k", "1")).await.unwrap().id,
        q.enqueue(&NewJob::new("q", "k", "2")).await.unwrap().id,
    ];
    let handler = Arc::new(Flaky::default());
    let worker = Worker::spawn(
        q.clone(),
        "q",
        "w",
        Arc::clone(&handler) as Arc<dyn Handler>,
        WorkerConfig {
            poll: Duration::from_millis(10),
            backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            ..WorkerConfig::default()
        },
    );
    for _ in 0..500 {
        if q.counts("q").await.unwrap().done == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    worker.abort();
    for id in ids {
        assert_eq!(q.status(id).await.unwrap().unwrap().0, State::Done);
    }
    assert_eq!(handler.0.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn rate_counts_add_up_across_replicas() {
    use vellum_db::rate::SharedCounts;
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(&IndexConfig::sqlite(
        dir.path().join("index.sqlite"),
        None,
    ))
    .await
    .unwrap();
    let counts = SharedCounts::new(index);
    let a = counts
        .add(60_000, &[("ip:1".to_owned(), 3), ("ip:2".to_owned(), 1)])
        .await
        .unwrap();
    assert!(a.contains(&("ip:1".to_owned(), 3)));
    // Another replica's hits for the same caller and window add up.
    let b = counts.add(60_000, &[("ip:1".to_owned(), 4)]).await.unwrap();
    assert_eq!(b, [("ip:1".to_owned(), 7)]);
    // A new window starts from zero.
    let c = counts
        .add(120_000, &[("ip:1".to_owned(), 1)])
        .await
        .unwrap();
    assert_eq!(c, [("ip:1".to_owned(), 1)]);
    assert_eq!(counts.prune(120_000).await.unwrap(), 2);
}
