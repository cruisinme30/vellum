// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `Index::reindex` on SQLite: the index is rebuilt from files alone.

use crate::common;

use std::path::Path;
use std::str::FromStr as _;

use sqlx::Connection as _;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use uuid::Uuid;
use vellum_db::{Index, IndexConfig, PlainFiles, ProblemKind, ReindexReport};
use vellum_kernel::ObjectId;

const fn uuid(id: ObjectId) -> Uuid {
    Uuid::from_u128(id.as_u128())
}

async fn connect(path: &Path) -> sqlx::Result<SqliteConnection> {
    let options = SqliteConnectOptions::from_str("sqlite://")?.filename(path);
    SqliteConnection::connect_with(&options).await
}

async fn count(conn: &mut SqliteConnection, sql: &'static str) -> sqlx::Result<i64> {
    sqlx::query_scalar(sql).fetch_one(conn).await
}

#[tokio::test]
async fn rebuilds_spaces_folders_documents_and_closure_from_files() {
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let db = tempfile::tempdir().unwrap();
    let path = db.path().join("index.sqlite");
    let index = Index::open(&IndexConfig::sqlite(path.clone(), None))
        .await
        .unwrap();

    let report = index.reindex(work.path(), PlainFiles).await.unwrap();
    assert_eq!((report.spaces, report.folders, report.documents), (3, 1, 9));
    let mut problems: Vec<_> = report
        .problems
        .iter()
        .map(|p| (p.path.as_str(), &p.kind))
        .collect();
    problems.sort_by_key(|p| p.0);
    assert_eq!(
        problems,
        [
            ("drives/shared/engineering/loose", &ProblemKind::MissingId),
            (
                "drives/shared/engineering/specs/untitled.md",
                &ProblemKind::MissingId
            ),
            (
                "drives/shared/engineering/specs/zz-roadmap-copy.md",
                &ProblemKind::DuplicateId(fx.roadmap)
            ),
        ]
    );

    let mut conn = connect(&path).await.unwrap();
    let kinds: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, path FROM documents ORDER BY kind, path")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    let kinds: Vec<(&str, &str)> = kinds
        .iter()
        .map(|(k, p)| (k.as_str(), p.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("book", "handbook.book"),
            ("collection", "okrs.collection"),
            ("issue", "issues/ENG-1.md"),
            ("page", "handbook.book/intro.md"),
            ("page", "loose/notes.md"),
            ("page", "specs/roadmap.md"),
            ("row", "okrs.collection/rows/q1.md"),
            ("shortcut", "go.shortcut.yaml"),
            ("task_list", "today.md"),
        ]
    );
    let roadmap: (String, String, Uuid) =
        sqlx::query_as("SELECT title, status, folder_id FROM documents WHERE doc_id = ?")
            .bind(uuid(fx.roadmap))
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(
        roadmap,
        (
            "Roadmap 2027".into(),
            "verified".into(),
            uuid(fx.specs_folder)
        )
    );

    // The book page sits under its book, then the space; every node has a self row.
    let chain: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT ancestor_id, depth FROM page_ancestors WHERE descendant_id = ? ORDER BY depth",
    )
    .bind(uuid(fx.handbook_intro))
    .fetch_all(&mut conn)
    .await
    .unwrap();
    assert_eq!(
        chain,
        [
            (uuid(fx.handbook_intro), 0),
            (uuid(fx.handbook), 1),
            (uuid(fx.eng_space), 2)
        ]
    );
    let nodes = count(
        &mut conn,
        "SELECT count(*) FROM page_ancestors WHERE depth = 0",
    )
    .await;
    assert_eq!(nodes.unwrap(), 3 + 1 + 9);
}

#[tokio::test]
async fn second_reindex_drops_deleted_files_and_keeps_grants() {
    let work = tempfile::tempdir().unwrap();
    let fx = common::northstar(work.path()).unwrap();
    let db = tempfile::tempdir().unwrap();
    let path = db.path().join("index.sqlite");
    let index = Index::open(&IndexConfig::sqlite(path.clone(), None))
        .await
        .unwrap();
    let _ = index.reindex(work.path(), PlainFiles).await.unwrap();

    let mut conn = connect(&path).await.unwrap();
    let _ = sqlx::query(
        "INSERT INTO grants (space_id, object_type, object_id, relation, subject_type, \
         subject_id, created_at) VALUES (?, 'doc', ?, 'viewer', 'user', ?, 0)",
    )
    .bind(uuid(fx.eng_space))
    .bind(uuid(fx.roadmap))
    .bind(Uuid::from_u128(7))
    .execute(&mut conn)
    .await
    .unwrap();
    std::fs::remove_file(work.path().join("drives/shared/engineering/loose/notes.md")).unwrap();

    let again = index.reindex(work.path(), PlainFiles).await.unwrap();
    assert_eq!(again.documents, 8);
    assert_eq!(
        count(&mut conn, "SELECT count(*) FROM documents")
            .await
            .unwrap(),
        8
    );
    assert_eq!(
        count(&mut conn, "SELECT count(*) FROM grants")
            .await
            .unwrap(),
        1
    );
    let stamp = "SELECT count(*) FROM index_meta WHERE key = 'last_reindex_ms'";
    assert_eq!(count(&mut conn, stamp).await.unwrap(), 1);
}

#[tokio::test]
async fn an_empty_folder_gives_an_empty_index() {
    let work = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    let index = Index::open(&IndexConfig::sqlite(db.path().join("i.sqlite"), None))
        .await
        .unwrap();
    let report = index.reindex(work.path(), PlainFiles).await.unwrap();
    assert_eq!(report, ReindexReport::default());
}

/// Every file under `root`, as a bucket listing would hold it.
fn listing_of(root: &Path) -> std::io::Result<vellum_db::Listing> {
    fn walk(root: &Path, dir: &Path, out: &mut vellum_db::Listing) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(std::io::Error::other)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let modified = std::fs::metadata(&path)?.modified()?;
            let bytes = if vellum_db::Listing::wants_content(&rel) {
                Some(std::fs::read(&path)?)
            } else {
                None
            };
            out.insert(&rel, modified, bytes);
        }
        Ok(())
    }
    let mut listing = vellum_db::Listing::new();
    walk(root, root, &mut listing)?;
    Ok(listing)
}

async fn rows(path: &Path) -> sqlx::Result<Vec<(String, String, String, i64)>> {
    let mut conn = connect(path).await?;
    sqlx::query_as("SELECT path, kind, content_hash, modified_at FROM documents ORDER BY path")
        .fetch_all(&mut conn)
        .await
}

#[tokio::test]
async fn a_bucket_listing_gives_the_same_index_as_the_folder() {
    let work = tempfile::tempdir().unwrap();
    let _fx = common::northstar(work.path()).unwrap();
    let db = tempfile::tempdir().unwrap();
    let from_disk = db.path().join("disk.sqlite");
    let from_bucket = db.path().join("bucket.sqlite");

    let disk = Index::open(&IndexConfig::sqlite(from_disk.clone(), None))
        .await
        .unwrap();
    let disk_report = disk.reindex(work.path(), PlainFiles).await.unwrap();
    disk.close().await;

    let listing = listing_of(work.path()).unwrap();
    assert!(!listing.is_empty());
    let bucket = Index::open(&IndexConfig::sqlite(from_bucket.clone(), None))
        .await
        .unwrap();
    let bucket_report = bucket.reindex_listing(listing).await.unwrap();
    bucket.close().await;

    assert_eq!(bucket_report, disk_report);
    let disk_rows = rows(&from_disk).await.unwrap();
    assert!(!disk_rows.is_empty());
    assert_eq!(rows(&from_bucket).await.unwrap(), disk_rows);
}

#[test]
fn only_pages_and_manifests_are_fetched() {
    use vellum_db::Listing;
    assert!(Listing::wants_content("drives/shared/eng/roadmap.md"));
    assert!(Listing::wants_content(
        "drives/shared/eng/.vellum-folder.yaml"
    ));
    assert!(Listing::wants_content(
        "drives/shared/eng/plan.book/book.yaml"
    ));
    assert!(!Listing::wants_content("drives/shared/eng/diagram.png"));
    assert!(!Listing::wants_content(".vellum/work/journal.json"));
    assert!(!Listing::wants_content(".vellum/versions/a.md"));
}
