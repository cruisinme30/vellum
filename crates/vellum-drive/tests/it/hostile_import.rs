// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Hostile imports (red team round 5, #259): an archive may carry files
//! with IDs Vellum already uses. An import must never take over them.

use std::sync::Arc;

use bytes::Bytes;
use vellum_drive::memory::{MemoryAccess, MemoryIndex};
use vellum_drive::{Drive, DriveIndex, SpaceKind};
use vellum_kernel::{Actor, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// RT-037: a shortcut, book or collection file in an import that names
/// another person's page ID gets a fresh ID instead of moving that page's
/// index entry into the importer's folder.
#[tokio::test]
async fn imported_ids_never_take_over_existing_nodes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index: Arc<dyn DriveIndex> = Arc::new(MemoryIndex::new());
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let drive = Drive::new(store, index.clone(), access);
    let alice = Actor::User {
        user: UserId::generate(),
    };
    let mallory = Actor::User {
        user: UserId::generate(),
    };
    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let secret = drive
        .create_page(&alice, eng.id, None, "Secret", "# Payroll\n")
        .await?;
    let book = drive.create_page(&alice, eng.id, None, "Other", "").await?;
    let own = drive
        .create_space(&mallory, SpaceKind::SharedDrive, "mine", "Mine", None)
        .await?;

    let files = vec![
        (
            "evil.shortcut.yaml".to_owned(),
            Bytes::from(format!(
                "id: {}\ntarget: {}\ntitle: Evil\n",
                secret.id, secret.id
            )),
        ),
        (
            "Evil.book/book.yaml".to_owned(),
            Bytes::from(format!("id: {}\ntitle: Evil\n", book.id)),
        ),
        ("Readme.md".to_owned(), Bytes::from_static(b"# Hi\n")),
    ];
    let imported = drive.import(&mallory, own.id, None, "Wiki", files).await?;
    assert_eq!(imported.nodes, 3, "{imported:?}");

    for (victim, path) in [(&secret, &secret.path), (&book, &book.path)] {
        let now = index.node(victim.id).await?.ok_or("victim gone")?;
        assert_eq!(now.space, eng.id, "the import took over {}", victim.id);
        assert_eq!(&now.path, path);
    }
    // The importer's copies live on under fresh IDs.
    let mine: Vec<_> = index
        .nodes(own.id)
        .await?
        .into_iter()
        .filter(|n| n.path.as_str().contains("Wiki"))
        .collect();
    assert!(mine.iter().all(|n| n.id != secret.id && n.id != book.id));
    Ok(())
}
