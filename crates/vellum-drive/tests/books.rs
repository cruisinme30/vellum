// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Books narrow access per section (`restrict_to` in `book.yaml`). A
//! person who can open a book must not get its restricted sections by
//! copying it, alone or inside a folder, nor through its snapshots, which
//! hold every section's full text, nor by reading its `book.yaml`
//! (issue #327, found by the red team).

use std::sync::Arc;

use bytes::Bytes;
use futures::TryStreamExt as _;
use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
use vellum_drive::{Drive, DriveIndex as _, NodeKind, SpaceKind};
use vellum_kernel::{Actor, ObjectId, UserId};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const SECRET: &str = "Alice 200k, Bob 90k";

async fn put(store: &LocalStore, path: &str, text: &str) -> TestResult {
    let _ = store
        .put(
            &BlobPath::new(path)?,
            Bytes::from(text.to_owned()),
            Precondition::None,
        )
        .await?;
    Ok(())
}

fn chapter(id: ObjectId, title: &str, body: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\nvellum: 1\n---\n\n# {title}\n\n{body}\n")
}

/// Writes `Handbook.book` under `dir`: a public section, and an HR
/// section (with a salaries page beneath it) only `hr` may see.
async fn handbook(store: &LocalStore, dir: &str, hr: UserId) -> TestResult {
    let (book, public, people, salaries) = (
        ObjectId::generate(),
        ObjectId::generate(),
        ObjectId::generate(),
        ObjectId::generate(),
    );
    let root = format!("{dir}/Handbook.book");
    put(
        store,
        &format!("{root}/book.yaml"),
        &format!(
            "vellum: 1\nid: {book}\ntitle: Handbook\nnodes:\n  {public}:\n    path: public.md\n  \
             {people}:\n    path: hr/README.md\n    restrict_to:\n      - user:{hr}\n  \
             {salaries}:\n    path: hr/salaries.md\n"
        ),
    )
    .await?;
    put(
        store,
        &format!("{root}/SUMMARY.md"),
        "# Summary\n\n- [Public](public.md)\n- [People](hr/README.md)\n  - [Salaries](hr/salaries.md)\n",
    )
    .await?;
    put(
        store,
        &format!("{root}/public.md"),
        &chapter(public, "Public", "Welcome to Northstar."),
    )
    .await?;
    put(
        store,
        &format!("{root}/hr/README.md"),
        &chapter(people, "People", "Only HR reads this."),
    )
    .await?;
    put(
        store,
        &format!("{root}/hr/salaries.md"),
        &chapter(salaries, "Salaries", SECRET),
    )
    .await?;
    put(
        store,
        &format!("{root}/.snapshots/v1.json"),
        &format!("{{\"tag\":\"v1\",\"sections\":[{{\"text\":\"{SECRET}\"}}]}}"),
    )
    .await?;
    Ok(())
}

/// Every file under `dir`, with its text.
async fn files_under(
    store: &LocalStore,
    dir: &BlobPath,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
    let entries: Vec<_> = store.list(Some(dir)).try_collect().await?;
    let mut out = Vec::new();
    for entry in entries {
        let bytes = store.get(&entry.path).await?.bytes;
        out.push((
            entry.path.as_str().to_owned(),
            String::from_utf8_lossy(&bytes).into_owned(),
        ));
    }
    Ok(out)
}

#[tokio::test]
async fn copies_and_reads_leave_restricted_sections_behind() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = Arc::new(MemoryIndex::new());
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let drive = Drive::new(store.clone(), index.clone(), access.clone());
    let (alice_id, bob_id) = (UserId::generate(), UserId::generate());
    let (alice, bob) = (Actor::User { user: alice_id }, Actor::User { user: bob_id });

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let docs = drive.create_folder(&alice, eng.id, None, "Docs").await?;
    handbook(&store, docs.path.as_str(), alice_id).await?;
    let _ = drive.scan().await?;
    let book = index
        .nodes(eng.id)
        .await?
        .into_iter()
        .find(|n| n.kind == NodeKind::Book)
        .ok_or("the book was not indexed")?;
    access.grant(bob_id, eng.id, Role::Viewer);
    let mine = drive
        .create_space(&bob, SpaceKind::UserDrive, "bob", "Bob", Some(bob_id))
        .await?;

    // Reading the book shows Bob the sections he may see, and nothing
    // about the restricted ones.
    let (_, manifest) = drive.read(&bob, book.id).await?;
    let manifest = String::from_utf8(manifest.bytes.to_vec())?;
    assert!(manifest.contains("public.md"), "{manifest}");
    assert!(!manifest.contains("hr/"), "{manifest}");
    assert!(!manifest.contains("restrict_to"), "{manifest}");

    // Copying the book, or the folder holding it, carries neither the
    // restricted sections nor the snapshots.
    for source in [book.id, docs.id] {
        let copy = drive.copy_node(&bob, source, mine.id, None).await?;
        let files = files_under(&store, &copy.path).await?;
        assert!(
            files.iter().any(|(p, _)| p.ends_with("public.md")),
            "{files:?}"
        );
        for (path, text) in &files {
            assert!(!text.contains(SECRET), "{path} leaks the salaries");
            assert!(!text.contains("Only HR"), "{path} leaks the HR section");
            assert!(!path.contains("/hr/"), "{path} was copied");
            assert!(!path.contains(".snapshots"), "{path} was copied");
        }
        let summary = files
            .iter()
            .find(|(p, _)| p.ends_with("SUMMARY.md"))
            .ok_or("no SUMMARY.md in the copy")?;
        assert!(!summary.1.contains("Salaries"), "{}", summary.1);
    }

    // Alice may see every section; her copy has them, still without
    // the snapshots.
    let copy = drive.copy_node(&alice, book.id, eng.id, None).await?;
    let files = files_under(&store, &copy.path).await?;
    assert!(files.iter().any(|(_, t)| t.contains(SECRET)), "{files:?}");
    assert!(!files.iter().any(|(p, _)| p.contains(".snapshots")));

    // A book that cannot be loaded is not copied at all.
    put(
        &store,
        &format!("{}/book.yaml", book.path.as_str()),
        "id: [not a book",
    )
    .await?;
    assert!(drive.copy_node(&bob, book.id, mine.id, None).await.is_err());
    Ok(())
}
