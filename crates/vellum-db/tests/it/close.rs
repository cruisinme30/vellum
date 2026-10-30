// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Closing an index never blocks the runtime it runs on.
//!
//! sqlx puts a released connection back from a task on the runtime that
//! released it, and `Pool::close` waits for that. Dropping the last index
//! on a current-thread runtime used to block the runtime's only thread
//! while it waited, so the close always timed out after 5 s and left the
//! connection to close while the process exited (#410).

use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use vellum_db::{Index, IndexConfig, IndexKey, IndexPool};

/// Far above a real close, far below the 5 s timeout the bug hit.
const QUICK: Duration = Duration::from_secs(1);

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// An encrypted index whose last query has just released its connection,
/// so sqlx's task to put it back is still queued on this runtime. Also
/// returns a handle on the pool, to watch it close.
async fn used_index(dir: &std::path::Path) -> TestResult<(Index, SqlitePool)> {
    let config = IndexConfig::sqlite(
        dir.join("index.sqlite"),
        Some(IndexKey::new([3; 32])),
    );
    let index = Index::open(&config).await?;
    let IndexPool::Sqlite(pool) = index.pool() else {
        return Err("a SQLite index has a SQLite pool".into());
    };
    let pool = pool.clone();
    index.ping().await?;
    Ok((index, pool))
}

/// Waits, letting this runtime run, until `done` holds or [`QUICK`] passes.
async fn within_quick(done: impl Fn() -> bool + Sync) -> TestResult {
    tokio::time::timeout(QUICK, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn closing_on_a_current_thread_runtime_is_quick() {
    let dir = tempfile::tempdir().unwrap();
    let (index, pool) = used_index(dir.path()).await.unwrap();
    let started = Instant::now();
    index.close().await;
    assert!(
        started.elapsed() < QUICK,
        "close took {:?}",
        started.elapsed()
    );
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn dropping_on_a_current_thread_runtime_does_not_block_it() {
    let dir = tempfile::tempdir().unwrap();
    let (index, pool) = used_index(dir.path()).await.unwrap();
    let started = Instant::now();
    drop(index);
    assert!(
        started.elapsed() < QUICK,
        "drop took {:?}",
        started.elapsed()
    );
    // The close the drop started finishes once the runtime runs again.
    within_quick(|| pool.is_closed() && pool.size() == 0)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn dropping_on_a_multi_thread_runtime_closes_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let (index, pool) = used_index(dir.path()).await.unwrap();
    let started = Instant::now();
    drop(index);
    assert!(
        started.elapsed() < QUICK,
        "drop took {:?}",
        started.elapsed()
    );
    assert!(pool.is_closed());
    assert_eq!(pool.size(), 0);
}

#[test]
fn dropping_outside_a_runtime_closes_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (index, pool) = runtime.block_on(used_index(dir.path())).unwrap();
    // Let the queued releases finish, as a runtime does before it stops.
    runtime
        .block_on(within_quick(|| {
            u32::try_from(pool.num_idle()) == Ok(pool.size())
        }))
        .unwrap();
    let started = Instant::now();
    drop(index);
    assert!(
        started.elapsed() < QUICK,
        "drop took {:?}",
        started.elapsed()
    );
    assert!(pool.is_closed());
    assert_eq!(pool.size(), 0);
}
