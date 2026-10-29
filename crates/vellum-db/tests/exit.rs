// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A process that drops encrypted indexes and exits straight away must exit
//! cleanly. Before the index closed its connections on drop, SQLCipher's
//! exit handler raced the connection threads and about one run in eight
//! crashed. The race only shows at process exit, so this test re-runs its
//! own binary as a child many times.

use std::process::{Command, ExitCode};

use vellum_db::{Index, IndexConfig, IndexKey};

const CHILD: &str = "VELLUM_DB_EXIT_CHILD";
const RUNS: usize = 60;

/// Opens a few encrypted indexes, uses them, drops them without `close`,
/// and returns, so the process exits at once.
fn child(dir: &str) -> ExitCode {
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        return ExitCode::FAILURE;
    };
    let worked = runtime.block_on(async {
        let mut indexes = Vec::new();
        for i in 0..4_u8 {
            let path = std::path::Path::new(dir).join(format!("i{i}.sqlite"));
            let config = IndexConfig::sqlite(path, Some(IndexKey::new([i; 32])));
            let Ok(index) = Index::open(&config).await else {
                return false;
            };
            indexes.push(index);
        }
        for index in &indexes {
            for _ in 0..4 {
                if index.ping().await.is_err() {
                    return false;
                }
            }
        }
        true
    });
    if worked {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a harness-less test reports on the terminal"
)]
fn main() -> ExitCode {
    if let Ok(dir) = std::env::var(CHILD) {
        return child(&dir);
    }
    let Ok(exe) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    let mut failures = Vec::new();
    for run in 0..RUNS {
        let Ok(dir) = tempfile::tempdir() else {
            return ExitCode::FAILURE;
        };
        let status = Command::new(&exe).env(CHILD, dir.path()).status();
        match status {
            Ok(status) if status.success() => {}
            other => failures.push(format!("run {run}: {other:?}")),
        }
    }
    if failures.is_empty() {
        println!("test drop_then_exit ... ok ({RUNS} runs)");
        ExitCode::SUCCESS
    } else {
        eprintln!("{} of {RUNS} runs did not exit cleanly:", failures.len());
        for failure in failures {
            eprintln!("  {failure}");
        }
        ExitCode::FAILURE
    }
}
