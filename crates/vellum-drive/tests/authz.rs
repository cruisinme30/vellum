// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive on the real permission engine (`vellum-authz`): owners,
//! deny by default, inheritance, moves and shortcuts.

use std::sync::Arc;

use vellum_authz::{
    Actor as AuthzActor, Authz, AuthzStore, Change, Id, MemoryStore, Object, Relation, SqlStore,
    Subject, Tuple, TupleRelation,
};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::{AuthzAccess, TreeSync};
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
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    the_drive_runs_on_rebac_with(Arc::new(MemoryIndex::new()), authz).await
}

#[tokio::test]
async fn the_drive_runs_on_rebac_with_the_sqlite_index() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    the_drive_runs_on_rebac_with(Arc::new(SqlIndex::new(db)), authz).await
}

/// As `vellum serve` runs it: the index and the permission engine share
/// one database, and the engine reads the tree the drive's index writes.
#[tokio::test]
async fn the_drive_runs_on_rebac_all_in_sqlite() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    let authz = Arc::new(Authz::new(SqlStore::new(db.clone())));
    the_drive_runs_on_rebac_with(Arc::new(SqlIndex::new(db)), authz).await
}

async fn the_drive_runs_on_rebac_with<S: AuthzStore + TreeSync + 'static>(
    index: Arc<dyn DriveIndex>,
    authz: Arc<Authz<S>>,
) -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
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

/// Copying a folder never hands over what a restriction hides inside it
/// (issue #322): a viewer of the folder could copy it into a drive they
/// own and read every restricted page.
#[tokio::test]
async fn copies_leave_restricted_parts_behind() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = Arc::new(MemoryIndex::new());
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let access = Arc::new(AuthzAccess::new(authz.clone(), index.clone()));
    let drive = Drive::new(store, index, access);
    let (alice_id, bob_id) = (UserId::generate(), UserId::generate());
    let (alice, bob) = (Actor::User { user: alice_id }, Actor::User { user: bob_id });

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let team = drive.create_folder(&alice, eng.id, None, "Team").await?;
    let hr = drive
        .create_folder(&alice, eng.id, Some(team.id), "HR only")
        .await?;
    let salaries = drive
        .create_page(&alice, eng.id, Some(hr.id), "Salaries", "Alice 200k")
        .await?;
    let notes = drive
        .create_page(&alice, eng.id, Some(team.id), "Notes", "public")
        .await?;
    let review = drive
        .create_page(&alice, eng.id, Some(team.id), "Review", "private")
        .await?;
    let _ = authz
        .write(
            space_id(eng.id),
            AuthzActor::User(Id(alice_id.as_u128())),
            &[
                grant(node(team.id), Relation::Viewer, bob_id),
                Change::Restrict(Id(hr.id.as_u128())),
                Change::Restrict(Id(review.id.as_u128())),
            ],
        )
        .await?;
    assert!(drive.read(&bob, notes.id).await.is_ok());
    for hidden in [salaries.id, review.id] {
        assert!(matches!(
            drive.read(&bob, hidden).await,
            Err(DriveError::PermissionDenied)
        ));
    }

    let mine = drive
        .create_space(&bob, SpaceKind::SharedDrive, "bob", "Bob", None)
        .await?;
    let copy = drive.copy_node(&bob, team.id, mine.id, None).await?;
    let titles: Vec<String> = drive
        .list(&bob, mine.id, Some(copy.id))
        .await?
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(titles, ["Notes"], "only what Bob can view is copied");

    // The owner still copies everything.
    let all = drive.copy_node(&alice, team.id, eng.id, None).await?;
    assert_eq!(drive.list(&alice, eng.id, Some(all.id)).await?.len(), 3);
    Ok(())
}

/// Moving or trashing a folder carries everything inside it, so a part
/// the person may not change blocks the whole operation, and nothing
/// moves. Without it, an editor of a folder could trash or move a page
/// restricted away from them.
#[tokio::test]
async fn folders_move_and_trash_only_with_rights_on_all_of_them() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = Arc::new(MemoryIndex::new());
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let access = Arc::new(AuthzAccess::new(authz.clone(), index.clone()));
    let drive = Drive::new(store, index, access);
    let (alice_id, bob_id) = (UserId::generate(), UserId::generate());
    let (alice, bob) = (Actor::User { user: alice_id }, Actor::User { user: bob_id });

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let team = drive.create_folder(&alice, eng.id, None, "Team").await?;
    let archive = drive.create_folder(&alice, eng.id, None, "Archive").await?;
    let notes = drive
        .create_page(&alice, eng.id, Some(team.id), "Notes", "public")
        .await?;
    let review = drive
        .create_page(&alice, eng.id, Some(team.id), "Review", "private")
        .await?;
    let _ = authz
        .write(
            space_id(eng.id),
            AuthzActor::User(Id(alice_id.as_u128())),
            &[
                grant(Object::Space(space_id(eng.id)), Relation::Editor, bob_id),
                Change::Restrict(Id(review.id.as_u128())),
            ],
        )
        .await?;
    assert!(matches!(
        drive.read(&bob, review.id).await,
        Err(DriveError::PermissionDenied)
    ));

    // Bob edits Team but not the page restricted inside it.
    assert!(matches!(
        drive.trash(&bob, team.id).await,
        Err(DriveError::PermissionDenied)
    ));
    assert!(matches!(
        drive
            .move_node(&bob, team.id, eng.id, Some(archive.id))
            .await,
        Err(DriveError::PermissionDenied)
    ));
    let still = drive.read(&alice, review.id).await?.0;
    assert!(still.is_live());
    assert_eq!(still.parent, Some(team.id), "nothing moved");

    // What Bob may change, he still moves and trashes.
    let moved = drive
        .move_node(&bob, notes.id, eng.id, Some(archive.id))
        .await?;
    assert_eq!(moved.parent, Some(archive.id));
    let _ = drive.trash(&bob, archive.id).await?;

    // Alice, who owns the space, moves and trashes Team whole.
    let old = drive.create_folder(&alice, eng.id, None, "Old").await?;
    let moved = drive
        .move_node(&alice, team.id, eng.id, Some(old.id))
        .await?;
    assert_eq!(moved.parent, Some(old.id));
    let _ = drive.trash(&alice, team.id).await?;
    Ok(())
}
