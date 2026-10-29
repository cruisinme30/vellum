// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive on the real permission engine (`vellum-authz`): owners,
//! deny by default, inheritance, moves and shortcuts.

use std::sync::Arc;

use vellum_authz::{
    Actor as AuthzActor, Authz, Change, Id, MemoryStore, Object, Relation, Subject, Tuple,
    TupleRelation,
};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::AuthzAccess;
use vellum_drive::memory::MemoryIndex;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveError, DriveIndex, SpaceKind};
use vellum_kernel::{Actor, ObjectId, SpaceId, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const fn grant(object: Object, relation: Relation, user: UserId) -> Change {
    Change::Grant(Tuple {
        object,
        relation: TupleRelation::Role(relation),
        subject: Subject::User(Id(user.as_u128())),
        expires_at: None,
    })
}

const fn node(id: ObjectId) -> Object {
    Object::Node(Id(id.as_u128()))
}

const fn space_id(id: SpaceId) -> Id {
    Id(id.as_u128())
}

#[tokio::test]
async fn the_drive_runs_on_rebac() -> TestResult {
    the_drive_runs_on_rebac_with(Arc::new(MemoryIndex::new())).await
}

#[tokio::test]
async fn the_drive_runs_on_rebac_with_the_sqlite_index() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    the_drive_runs_on_rebac_with(Arc::new(SqlIndex::new(db))).await
}

async fn the_drive_runs_on_rebac_with(index: Arc<dyn DriveIndex>) -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let access = Arc::new(AuthzAccess::new(authz.clone(), index.clone()));
    let drive = Drive::new(store, index, access);
    let (alice_id, bob_id) = (UserId::generate(), UserId::generate());
    let (alice, bob) = (Actor::User { user: alice_id }, Actor::User { user: bob_id });

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let a = drive.create_folder(&alice, eng.id, None, "A").await?;
    let b = drive.create_folder(&alice, eng.id, None, "B").await?;
    let page = drive
        .create_page(&alice, eng.id, Some(a.id), "Plan", "x")
        .await?;
    assert!(drive.read(&alice, page.id).await.is_ok());
    assert!(matches!(
        drive.read(&bob, page.id).await,
        Err(DriveError::PermissionDenied)
    ));

    // A viewer grant on A reaches the page; Bob still cannot edit or purge.
    let _ = authz
        .write(
            space_id(eng.id),
            AuthzActor::User(Id(alice_id.as_u128())),
            &[grant(node(a.id), Relation::Viewer, bob_id)],
        )
        .await?;
    assert!(drive.read(&bob, page.id).await.is_ok());
    assert!(matches!(
        drive.rename(&bob, page.id, "Nope").await,
        Err(DriveError::PermissionDenied)
    ));
    assert_eq!(drive.shared_drives(bob_id).await?.len(), 1);

    // Moving the page out of A drops the inherited view.
    let moved = drive.move_node(&alice, page.id, eng.id, Some(b.id)).await?;
    assert!(matches!(
        drive.read(&bob, moved.id).await,
        Err(DriveError::PermissionDenied)
    ));

    // An explicit grant on the page survives the next move.
    let _ = authz
        .write(
            space_id(eng.id),
            AuthzActor::User(Id(alice_id.as_u128())),
            &[grant(node(page.id), Relation::Editor, bob_id)],
        )
        .await?;
    let moved = drive.move_node(&alice, page.id, eng.id, Some(a.id)).await?;
    assert!(drive.rename(&bob, moved.id, "Plan v2").await.is_ok());

    // A shortcut in a folder Bob can see does not open what it points at.
    let secret = drive
        .create_page(&alice, eng.id, Some(b.id), "Secret", "")
        .await?;
    let shortcut = drive
        .create_shortcut(&alice, secret.id, eng.id, Some(a.id))
        .await?;
    assert!(drive.get(&bob, shortcut.id).await.is_ok());
    assert!(matches!(
        drive.resolve_shortcut(&bob, shortcut.id).await,
        Err(DriveError::PermissionDenied)
    ));

    // Trash and restore keep access in line with the tree.
    let _ = drive.trash(&alice, a.id).await?;
    assert!(drive.restore(&alice, a.id).await.is_ok());
    assert!(drive.read(&bob, moved.id).await.is_ok());
    Ok(())
}
