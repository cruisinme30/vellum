// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Concurrent page saves on the built-in SQLite index never fail with
//! "database is locked" (#542, D306).
//!
//! Every save runs write transactions that read before they write. On
//! SQLite in WAL mode a deferred `BEGIN` makes such a transaction fail at
//! once with `SQLITE_BUSY_SNAPSHOT` when another connection commits
//! between its read and its write; the busy timeout cannot help. Write
//! transactions now start with `BEGIN IMMEDIATE` and wait their turn.

use std::sync::Arc;

use bytes::Bytes;
use vellum_db::{Index, IndexConfig};
use vellum_drive::memory::MemoryAccess;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, SpaceKind};
use vellum_kernel::{Actor, UserId};
use vellum_store::{Base, BlobStore as _, LocalStore};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Writers saving at once; more than the pool's four connections.
const WRITERS: usize = 16;
/// Saves per writer.
const SAVES: usize = 25;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_page_saves_never_hit_a_locked_database() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(
        dir.path().join("index.sqlite"),
        None,
    ))
    .await?;
    let store = Arc::new(LocalStore::open(dir.path().join("files")).await?);
    let index = Arc::new(SqlIndex::new(db));
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let drive = Arc::new(Drive::new(store.clone(), index, access));
    let alice = Actor::User {
        user: UserId::generate(),
    };
    let space = drive
        .create_space(
            &alice,
            SpaceKind::SharedDrive,
            "eng",
            "Engineering",
            None,
        )
        .await?;

    let mut pages = Vec::with_capacity(WRITERS);
    for n in 0..WRITERS {
        let title = format!("Page {n}");
        pages.push(
            drive
                .create_page(&alice, space.id, None, &title, "start\n")
                .await?,
        );
    }

    let mut tasks = Vec::with_capacity(WRITERS);
    for page in pages {
        let (drive, store) = (drive.clone(), store.clone());
        tasks.push(tokio::spawn(async move {
            let mut failures = Vec::new();
            for round in 0..SAVES {
                let blob = store.get(&page.path).await?;
                let base = Base {
                    bytes: blob.bytes.clone(),
                    etag: Some(blob.meta.etag.clone()),
                };
                let content = Bytes::from(format!("round {round}\n"));
                if let Err(error) =
                    drive.save(&alice, page.id, base, content).await
                {
                    failures.push(error.to_string());
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(failures)
        }));
    }
    let mut failures = Vec::new();
    for task in tasks {
        failures.extend(task.await??);
    }
    assert!(
        failures.is_empty(),
        "{} of {} saves failed, first: {:?}",
        failures.len(),
        WRITERS * SAVES,
        failures.first()
    );
    Ok(())
}
