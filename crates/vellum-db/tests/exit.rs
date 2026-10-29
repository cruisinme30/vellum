// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A process that drops encrypted indexes and exits straight away must exit
//! cleanly. Before the index closed its connections on drop, SQLCipher's
//! exit handler raced the connection threads and about one run in eight
//! crashed. The race only shows at process exit, so `drop_then_exit`
//! re-runs this test binary many times, each time running only the ignored
//! `child_process` test.

use std::process::Command;

use vellum_db::{Index, IndexConfig, IndexKey};

const CHILD: &str = "VELLUM_DB_EXIT_CHILD";
const RUNS: usize = 60;

/// The child: opens a few encrypted indexes, uses them, and drops them
/// without `close`. The process exits as soon as this returns. It does
/// nothing unless the parent test started it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "run by drop_then_exit in a child process"]
async fn child_process() {
    let Ok(dir) = std::env::var(CHILD) else {
        return;
    };
    let mut indexes = Vec::new();
    for i in 0..4_u8 {
        let path = std::path::Path::new(&dir).join(format!("i{i}.sqlite"));
        let config = IndexConfig::sqlite(path, Some(IndexKey::new([i; 32])));
        indexes.push(Index::open(&config).await.unwrap());
    }
    for index in &indexes {
        for _ in 0..4 {
            index.ping().await.unwrap();
        }
    }
}

#[test]
fn drop_then_exit() {
    let exe = std::env::current_exe().unwrap();
    let mut failures = Vec::new();
    for run in 0..RUNS {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new(&exe)
            .args(["child_process", "--exact", "--ignored", "--quiet"])
            .env(CHILD, dir.path())
            .output();
        match status {
            Ok(out) if out.status.success() => {}
            Ok(out) => failures.push(format!("run {run}: {}", out.status)),
            Err(error) => failures.push(format!("run {run}: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {RUNS} runs did not exit cleanly: {failures:?}",
        failures.len()
    );
}
