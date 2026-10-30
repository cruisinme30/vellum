// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The watcher reports external edits by identity: renames are detected
//! by ULID, copies are flagged, and offline edits are found at startup.

use std::time::{Duration, Instant};

use bytes::Bytes;
use vellum_kernel::ObjectId;
use vellum_store::{
    BlobPath, BlobStore, LocalStore, Precondition,
    watch::{Change, LocalFeed, RENAME_GRACE, Reconciler, run},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const A: &str = "01JBQ3Z8V9G6Q2W4E5R7T8Y9XA";
const B: &str = "01JBQ3Z8V9G6Q2W4E5R7T8Y9XB";

fn doc(id: &str, body: &str) -> Bytes {
    Bytes::from(format!("---\nid: {id}\ntitle: T\n---\n{body}\n"))
}

#[expect(clippy::unwrap_used, reason = "test helper over literal paths")]
fn p(s: &str) -> BlobPath {
    BlobPath::new(s).unwrap()
}

const fn kind(change: &Change) -> &'static str {
    match change {
        Change::Created { .. } => "created",
        Change::Modified { .. } => "modified",
        Change::Renamed { .. } => "renamed",
        Change::Removed { .. } => "removed",
        Change::Duplicated { .. } => "duplicated",
        Change::Untracked { .. } => "untracked",
    }
}

async fn store() -> Result<(tempfile::TempDir, LocalStore), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = LocalStore::open(dir.path()).await?;
    Ok((dir, store))
}

#[tokio::test]
async fn startup_scan_finds_offline_edits() -> TestResult {
    let (_dir, store) = store().await?;
    let (a, b): (ObjectId, ObjectId) = (A.parse()?, B.parse()?);
    let kept = store
        .put(
            &p("drives/users/ana/Kept.md"),
            doc(A, "v1"),
            Precondition::None,
        )
        .await?;
    let moved = store
        .put(
            &p("drives/users/ana/Old.md"),
            doc(B, "v1"),
            Precondition::None,
        )
        .await?;
    let known = vec![
        (a, kept.path.clone(), kept.etag),
        (b, moved.path.clone(), moved.etag),
    ];
    let mut watcher = Reconciler::new(store.clone(), known);

    // While Vellum was down: A edited, B renamed, C created, an asset added.
    let _ = store
        .put(&kept.path, doc(A, "v2"), Precondition::None)
        .await?;
    let _ = store
        .rename(&moved.path, &p("drives/users/ana/New.md"))
        .await?;
    let c = "01JBQ3Z8V9G6Q2W4E5R7T8Y9XC";
    let _ = store
        .put(&p("drives/users/ana/C.md"), doc(c, "x"), Precondition::None)
        .await?;
    let _ = store
        .put(
            &p("drives/users/ana/logo.png"),
            Bytes::from_static(b"png"),
            Precondition::None,
        )
        .await?;
    let _ = store
        .put(
            &p(".vellum/index.sqlite"),
            Bytes::from_static(b"db"),
            Precondition::None,
        )
        .await?;

    let changes = watcher.scan().await?;
    let mut kinds: Vec<&str> = changes.iter().map(kind).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        ["created", "modified", "renamed", "untracked"],
        "{changes:#?}"
    );
    assert!(changes.iter().any(|c| matches!(c, Change::Renamed { id, to, .. } if *id == b && to.as_str() == "drives/users/ana/New.md")));
    assert!(
        watcher
            .scan()
            .await?
            .iter()
            .all(|c| matches!(c, Change::Untracked { .. })),
        "second scan is quiet"
    );
    Ok(())
}

#[tokio::test]
async fn rename_split_across_batches_is_one_rename() -> TestResult {
    let (_dir, store) = store().await?;
    let id: ObjectId = A.parse()?;
    let meta = store
        .put(&p("a/Plan.md"), doc(A, "v1"), Precondition::None)
        .await?;
    let mut watcher = Reconciler::new(store.clone(), [(id, meta.path.clone(), meta.etag)]);
    let _ = store.rename(&meta.path, &p("b/Plan.md")).await?;

    let now = Instant::now();
    assert_eq!(watcher.reconcile([p("a/Plan.md")], now).await?, []);
    let later = watcher
        .reconcile([p("b/Plan.md")], now + Duration::from_millis(500))
        .await?;
    assert!(
        matches!(&later[..], [Change::Renamed { from, to, .. }] if from.as_str() == "a/Plan.md" && to.as_str() == "b/Plan.md"),
        "{later:?}"
    );
    assert_eq!(
        watcher.reconcile([], now + RENAME_GRACE * 2).await?,
        [],
        "no late removal"
    );
    Ok(())
}

#[tokio::test]
async fn removal_waits_for_the_grace_period() -> TestResult {
    let (_dir, store) = store().await?;
    let id: ObjectId = A.parse()?;
    let meta = store
        .put(&p("Plan.md"), doc(A, "v1"), Precondition::None)
        .await?;
    let mut watcher = Reconciler::new(store.clone(), [(id, meta.path.clone(), meta.etag)]);
    store.delete(&meta.path, Precondition::None).await?;
    let now = Instant::now();
    assert_eq!(watcher.reconcile([meta.path.clone()], now).await?, []);
    assert!(watcher.next_deadline().is_some());
    let due = watcher.reconcile([], now + RENAME_GRACE).await?;
    assert_eq!(
        due,
        [Change::Removed {
            id,
            path: meta.path
        }]
    );
    Ok(())
}

#[tokio::test]
async fn own_writes_are_silent_and_copies_are_flagged() -> TestResult {
    let (_dir, store) = store().await?;
    let id: ObjectId = A.parse()?;
    let v1 = store
        .put(&p("Plan.md"), doc(A, "v1"), Precondition::None)
        .await?;
    let mut watcher = Reconciler::new(store.clone(), [(id, v1.path.clone(), v1.etag)]);
    let v2 = store
        .put(&v1.path, doc(A, "v2"), Precondition::None)
        .await?;
    watcher.record(id, v2.path.clone(), v2.etag);
    assert_eq!(
        watcher.reconcile([v1.path.clone()], Instant::now()).await?,
        []
    );

    let _ = store
        .put(&p("Plan copy.md"), doc(A, "v2"), Precondition::None)
        .await?;
    let changes = watcher
        .reconcile([p("Plan copy.md")], Instant::now())
        .await?;
    assert_eq!(
        changes,
        [Change::Duplicated {
            id,
            original: v1.path,
            copy: p("Plan copy.md")
        }]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn live_feed_reports_external_edits() -> TestResult {
    let (dir, store) = store().await?;
    let (paths_tx, paths_rx) = tokio::sync::mpsc::channel(64);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel(64);
    let _feed = LocalFeed::start(dir.path(), paths_tx)?;
    let task = tokio::spawn(run(Reconciler::new(store, []), paths_rx, out_tx));

    std::fs::create_dir_all(dir.path().join("drives"))?;
    std::fs::write(dir.path().join("drives/Plan.md"), doc(A, "v1"))?;
    let created = tokio::time::timeout(Duration::from_secs(10), out_rx.recv()).await?;
    assert!(
        matches!(created, Some(Change::Created { ref path, .. }) if path.as_str() == "drives/Plan.md"),
        "{created:?}"
    );

    std::fs::rename(
        dir.path().join("drives/Plan.md"),
        dir.path().join("drives/Roadmap.md"),
    )?;
    let renamed = tokio::time::timeout(Duration::from_secs(10), out_rx.recv()).await?;
    assert!(
        matches!(renamed, Some(Change::Renamed { ref to, .. }) if to.as_str() == "drives/Roadmap.md"),
        "{renamed:?}"
    );

    // Moving a whole folder is reported for the folder only; the
    // document inside must still come out as a rename.
    std::fs::rename(dir.path().join("drives"), dir.path().join("moved"))?;
    let moved = tokio::time::timeout(Duration::from_secs(10), out_rx.recv()).await?;
    assert!(
        matches!(moved, Some(Change::Renamed { ref to, .. }) if to.as_str() == "moved/Roadmap.md"),
        "{moved:?}"
    );
    task.abort();
    Ok(())
}
