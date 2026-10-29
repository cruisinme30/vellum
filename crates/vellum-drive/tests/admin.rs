// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What workspace administrators do to spaces (ADR 0021, D242), on the
//! real permission engine.

#![allow(
    clippy::too_many_lines,
    reason = "one test walks an admin's whole visit"
)]

use std::sync::Arc;

use vellum_authz::{Authz, AuthzStore, MemoryStore, SqlStore};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::{AuthzAccess, TreeSync};
use vellum_drive::memory::MemoryIndex;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveError, DriveIndex, Member, SpaceKind, SpaceRole};
use vellum_kernel::{Actor, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn admins_manage_spaces_in_memory() -> TestResult {
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    admins_manage_spaces(Arc::new(MemoryIndex::new()), authz).await
}

/// As `vellum serve` runs it: index and engine in one database.
#[tokio::test]
async fn admins_manage_spaces_in_sqlite() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    let authz = Arc::new(Authz::new(SqlStore::new(db.clone())));
    admins_manage_spaces(Arc::new(SqlIndex::new(db)), authz).await
}

async fn admins_manage_spaces<S: AuthzStore + TreeSync + 'static>(
    index: Arc<dyn DriveIndex>,
    authz: Arc<Authz<S>>,
) -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let access = Arc::new(AuthzAccess::new(authz, index.clone()));
    let drive = Drive::new(store, index, access);
    let (priya, bob) = (UserId::generate(), UserId::generate());
    let as_bob = Actor::User { user: bob };

    // An admin creates a shared drive for Priya, who owns it at once.
    let eng = drive
        .admin_create_space("eng", "Engineering", priya)
        .await?;
    assert_eq!(
        eng.members,
        vec![Member {
            user: priya,
            role: SpaceRole::Owner
        }]
    );
    let page = drive
        .create_page(
            &Actor::User { user: priya },
            eng.space.id,
            None,
            "Plan",
            "hello",
        )
        .await?;

    // Bob sees nothing until an admin adds him.
    assert!(matches!(
        drive.read(&as_bob, page.id).await,
        Err(DriveError::PermissionDenied)
    ));
    let _ = drive
        .admin_set_member(eng.space.id, bob, Some(SpaceRole::Viewer))
        .await?;
    assert!(drive.read(&as_bob, page.id).await.is_ok());
    assert!(matches!(
        drive.rename(&as_bob, page.id, "Mine").await,
        Err(DriveError::PermissionDenied)
    ));

    // A new role replaces the old one rather than adding to it.
    let changed = drive
        .admin_set_member(eng.space.id, bob, Some(SpaceRole::Editor))
        .await?;
    assert!(changed.members.contains(&Member {
        user: bob,
        role: SpaceRole::Editor
    }));
    assert_eq!(changed.members.len(), 2);
    assert!(drive.rename(&as_bob, page.id, "Plan v2").await.is_ok());

    // The drive always keeps someone who manages it.
    assert!(matches!(
        drive.admin_set_member(eng.space.id, priya, None).await,
        Err(DriveError::Invalid(_))
    ));
    assert!(matches!(
        drive
            .admin_set_member(eng.space.id, priya, Some(SpaceRole::Editor))
            .await,
        Err(DriveError::Invalid(_))
    ));
    let _ = drive
        .admin_set_member(eng.space.id, bob, Some(SpaceRole::Manager))
        .await?;
    let after = drive.admin_set_member(eng.space.id, priya, None).await?;
    assert_eq!(
        after.members,
        vec![Member {
            user: bob,
            role: SpaceRole::Manager
        }]
    );
    // Bob manages it now. Priya keeps the page she made: its creator owns it.
    let renamed = drive
        .update_space(
            &as_bob,
            eng.space.id,
            Some("Eng"),
            eng.space.settings.clone(),
        )
        .await?;
    assert_eq!(renamed.name, "Eng");

    // A My Drive is its owner's: an admin sets its quota but adds nobody.
    let mine = drive.ensure_my_drive(priya, Some("priya")).await?;
    assert!(matches!(
        drive
            .admin_set_member(mine.id, bob, Some(SpaceRole::Viewer))
            .await,
        Err(DriveError::Invalid(_))
    ));
    let mut settings = mine.settings.clone();
    settings.quota_bytes = Some(1_000_000);
    let updated = drive.admin_update_space(mine.id, None, settings).await?;
    assert_eq!(updated.space.settings.quota_bytes, Some(1_000_000));
    assert_eq!(updated.space.name, mine.name);
    assert!(matches!(
        drive
            .admin_update_space(mine.id, Some("  "), mine.settings.clone())
            .await,
        Err(DriveError::Invalid(_))
    ));

    // The list shows every space, shared drives first, with their size.
    let all = drive.admin_spaces().await?;
    let kinds: Vec<SpaceKind> = all.iter().map(|a| a.space.kind).collect();
    assert_eq!(kinds, vec![SpaceKind::SharedDrive, SpaceKind::UserDrive]);
    assert!(
        all.first().is_some_and(|a| a.used > 0),
        "the page counts towards the drive's size"
    );
    Ok(())
}
