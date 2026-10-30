// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Scenario tests for the authorization service over the memory adapter.

// Test helpers outside #[test] functions may unwrap like the tests do.
#![allow(
    clippy::unwrap_used,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use vellum_authz::{
    Actor, Authz, AuthzStore as _, Change, Commit, Error, Id, MemoryStore,
    Object, Permission, Principal, Relation, Scope, StoreError, Subject, Tuple,
};

const SPACE: Id = Id(0x5000);
const OTHER_SPACE: Id = Id(0x6000);
const ENG: Id = Id(0x100); // folder "Engineering"
const RUNBOOKS: Id = Id(0x101); // folder "Runbooks"
const SPEC: Id = Id(0x200); // doc in ENG
const ONCALL: Id = Id(0x201); // doc in RUNBOOKS
const SHORTCUT: Id = Id(0x202); // ENG/oncall.shortcut.yaml → ONCALL

const PRIYA: Id = Id(1);
const MARCUS: Id = Id(2);
const LENA: Id = Id(3);
const PLATFORM: Id = Id(0x900); // group
const ALL_STAFF: Id = Id(0x901); // group containing PLATFORM
const LINK: Id = Id(0x700); // share link

fn user(id: Id) -> Principal {
    Principal::User(id)
}

fn tree() -> BTreeMap<Id, Option<Id>> {
    BTreeMap::from([
        (ENG, None),
        (RUNBOOKS, None),
        (SPEC, Some(ENG)),
        (ONCALL, Some(RUNBOOKS)),
        (SHORTCUT, Some(ENG)),
    ])
}

async fn setup() -> Authz<MemoryStore> {
    let store = MemoryStore::new();
    store.set_tree(SPACE, tree()).unwrap();
    let authz = Authz::new(store);
    let _ = authz.rebuild(SPACE).await.unwrap();
    authz
}

fn grant(object: Object, relation: Relation, subject: Subject) -> Change {
    Change::Grant(Tuple::role(object, relation, subject))
}

async fn can(
    authz: &Authz<MemoryStore>,
    who: Principal,
    object: Id,
    p: Permission,
) -> bool {
    authz.check(SPACE, who, object, p).await.unwrap().allowed
}

#[tokio::test]
async fn denies_by_default() {
    let authz = setup().await;
    for object in [SPACE, ENG, SPEC, ONCALL, Id(0xdead)] {
        assert!(!can(&authz, user(PRIYA), object, Permission::Read).await);
    }
    let d = authz
        .check(OTHER_SPACE, user(PRIYA), SPEC, Permission::Read)
        .await
        .unwrap();
    assert_eq!(d.relation, None);
    assert!(
        authz
            .accessible(SPACE, user(PRIYA), Permission::Read)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn folder_grants_are_inherited_and_revocation_is_immediate() {
    let authz = setup().await;
    let before = authz.epoch(SPACE).await.unwrap();
    let e1 = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(ENG),
                Relation::Editor,
                Subject::User(PRIYA),
            )],
        )
        .await
        .unwrap();
    assert_eq!(e1, before + 1);
    let d = authz
        .check(SPACE, user(PRIYA), SPEC, Permission::Edit)
        .await
        .unwrap();
    assert!(d.allowed);
    assert_eq!(d.relation, Some(Relation::Editor));
    assert_eq!(d.epoch, Some(e1));
    assert!(!can(&authz, user(PRIYA), SPEC, Permission::Share).await);
    assert!(!can(&authz, user(PRIYA), ONCALL, Permission::Read).await);
    assert!(!can(&authz, user(PRIYA), SPACE, Permission::Read).await);

    let e2 = authz
        .write(
            SPACE,
            Actor::System,
            &[Change::Revoke {
                object: Object::Node(ENG),
                relation: Relation::Editor,
                subject: Subject::User(PRIYA),
            }],
        )
        .await
        .unwrap();
    assert_eq!(e2, e1 + 1);
    assert!(!can(&authz, user(PRIYA), SPEC, Permission::Read).await);
}

#[tokio::test]
async fn space_grants_reach_every_node() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Space(SPACE),
                Relation::Viewer,
                Subject::User(LENA),
            )],
        )
        .await
        .unwrap();
    let mut readable: Vec<Id> = authz
        .accessible(SPACE, user(LENA), Permission::Read)
        .await
        .unwrap();
    readable.sort();
    let mut expected = vec![SPACE, ENG, RUNBOOKS, SPEC, ONCALL, SHORTCUT];
    expected.sort();
    assert_eq!(readable, expected);
    assert!(
        authz
            .accessible(SPACE, user(LENA), Permission::Edit)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn moved_doc_keeps_explicit_grants_and_drops_inherited() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[
                grant(
                    Object::Node(ENG),
                    Relation::Viewer,
                    Subject::User(PRIYA),
                ),
                grant(
                    Object::Node(SPEC),
                    Relation::Editor,
                    Subject::User(MARCUS),
                ),
                grant(
                    Object::Node(RUNBOOKS),
                    Relation::Commenter,
                    Subject::User(LENA),
                ),
            ],
        )
        .await
        .unwrap();
    assert!(can(&authz, user(PRIYA), SPEC, Permission::Read).await);

    // The index moves SPEC from Engineering to Runbooks.
    let mut moved = tree();
    let _ = moved.insert(SPEC, Some(RUNBOOKS));
    authz.store().set_tree(SPACE, moved).unwrap();
    let _ = authz.tree_changed(SPACE, &[SPEC]).await.unwrap();

    assert!(
        !can(&authz, user(PRIYA), SPEC, Permission::Read).await,
        "inherited grant dropped"
    );
    assert!(
        can(&authz, user(MARCUS), SPEC, Permission::Edit).await,
        "explicit grant kept"
    );
    assert!(
        can(&authz, user(LENA), SPEC, Permission::Comment).await,
        "new parent inherited"
    );
}

#[tokio::test]
async fn deleted_nodes_lose_their_rows() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Space(SPACE),
                Relation::Viewer,
                Subject::User(LENA),
            )],
        )
        .await
        .unwrap();
    let mut pruned = tree();
    let _ = pruned.remove(&SPEC);
    authz.store().set_tree(SPACE, pruned).unwrap();
    let _ = authz.tree_changed(SPACE, &[SPEC]).await.unwrap();
    assert!(!can(&authz, user(LENA), SPEC, Permission::Read).await);
    assert!(can(&authz, user(LENA), ENG, Permission::Read).await);
}

#[tokio::test]
async fn shortcuts_never_grant_access_to_their_target() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(ENG),
                Relation::Manager,
                Subject::User(PRIYA),
            )],
        )
        .await
        .unwrap();
    assert!(can(&authz, user(PRIYA), SHORTCUT, Permission::Read).await);
    assert!(!can(&authz, user(PRIYA), ONCALL, Permission::Read).await);
    // Even an explicit grant on the shortcut object stays on the shortcut.
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(SHORTCUT),
                Relation::Owner,
                Subject::User(MARCUS),
            )],
        )
        .await
        .unwrap();
    assert!(!can(&authz, user(MARCUS), ONCALL, Permission::Read).await);
}

#[tokio::test]
async fn users_cannot_hand_out_more_than_they_hold() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[
                grant(
                    Object::Node(ENG),
                    Relation::Editor,
                    Subject::User(PRIYA),
                ),
                grant(
                    Object::Node(ENG),
                    Relation::Manager,
                    Subject::User(MARCUS),
                ),
                grant(
                    Object::Space(SPACE),
                    Relation::Owner,
                    Subject::User(LENA),
                ),
            ],
        )
        .await
        .unwrap();
    let to_viewer = |s| {
        [grant(
            Object::Node(SPEC),
            Relation::Viewer,
            Subject::User(s),
        )]
    };

    // An editor cannot share.
    let err = authz
        .write(SPACE, Actor::User(PRIYA), &to_viewer(Id(9)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden { .. }));
    // A manager can share within the folder, but not make owners.
    let _ = authz
        .write(SPACE, Actor::User(MARCUS), &to_viewer(Id(9)))
        .await
        .unwrap();
    let owner = [grant(
        Object::Node(SPEC),
        Relation::Owner,
        Subject::User(Id(9)),
    )];
    assert!(matches!(
        authz.write(SPACE, Actor::User(MARCUS), &owner).await,
        Err(Error::Forbidden { .. })
    ));
    // ...and not outside the folder.
    let outside = [grant(
        Object::Node(ONCALL),
        Relation::Viewer,
        Subject::User(Id(9)),
    )];
    assert!(matches!(
        authz.write(SPACE, Actor::User(MARCUS), &outside).await,
        Err(Error::Forbidden { .. })
    ));
    // Nobody can restrict without manager.
    assert!(matches!(
        authz
            .write(SPACE, Actor::User(PRIYA), &[Change::Restrict(SPEC)])
            .await,
        Err(Error::Forbidden { .. })
    ));
    // An owner can do all of it.
    let _ = authz.write(SPACE, Actor::User(LENA), &owner).await.unwrap();
    assert!(
        can(&authz, user(Id(9)), SPEC, Permission::TransferOwnership).await
    );
}

#[tokio::test]
async fn group_membership_changes_reach_every_granting_space() {
    let store = MemoryStore::new();
    store.set_tree(SPACE, tree()).unwrap();
    store
        .set_tree(OTHER_SPACE, BTreeMap::from([(Id(0x300), None)]))
        .unwrap();
    let authz = Authz::new(store);
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(ENG),
                Relation::Editor,
                Subject::Group(ALL_STAFF),
            )],
        )
        .await
        .unwrap();
    let _ = authz
        .write(
            OTHER_SPACE,
            Actor::System,
            &[grant(
                Object::Space(OTHER_SPACE),
                Relation::Viewer,
                Subject::Group(PLATFORM),
            )],
        )
        .await
        .unwrap();

    // PLATFORM ⊂ ALL_STAFF; Priya joins PLATFORM.
    authz
        .update_group(ALL_STAFF, &[Subject::Group(PLATFORM)], &[])
        .await
        .unwrap();
    authz
        .update_group(PLATFORM, &[Subject::User(PRIYA)], &[])
        .await
        .unwrap();
    assert!(can(&authz, user(PRIYA), SPEC, Permission::Edit).await);
    assert!(
        authz
            .check(OTHER_SPACE, user(PRIYA), Id(0x300), Permission::Read)
            .await
            .unwrap()
            .allowed
    );
    let mut spaces = authz.spaces(user(PRIYA)).await.unwrap();
    spaces.sort();
    assert_eq!(spaces, vec![SPACE, OTHER_SPACE]);

    // Un-nesting PLATFORM removes the ALL_STAFF grant but keeps the direct one.
    authz
        .update_group(ALL_STAFF, &[], &[Subject::Group(PLATFORM)])
        .await
        .unwrap();
    assert!(!can(&authz, user(PRIYA), SPEC, Permission::Read).await);
    assert!(
        authz
            .check(OTHER_SPACE, user(PRIYA), Id(0x300), Permission::Read)
            .await
            .unwrap()
            .allowed
    );

    authz
        .update_group(PLATFORM, &[], &[Subject::User(PRIYA)])
        .await
        .unwrap();
    assert!(authz.spaces(user(PRIYA)).await.unwrap().is_empty());
}

#[tokio::test]
async fn share_links_are_principals_of_their_own() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(SPEC),
                Relation::Commenter,
                Subject::ShareLink(LINK),
            )],
        )
        .await
        .unwrap();
    let link = Principal::ShareLink(LINK);
    assert!(can(&authz, link, SPEC, Permission::Comment).await);
    assert!(!can(&authz, link, SPEC, Permission::Edit).await);
    assert!(!can(&authz, link, ENG, Permission::Read).await);
}

#[tokio::test]
async fn restricted_book_sections_narrow_access() {
    // A book (ENG) with a restricted section (SPEC): only explicit grantees
    // keep access, capped at their book-level role; managers stay.
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[
                grant(
                    Object::Node(ENG),
                    Relation::Editor,
                    Subject::User(PRIYA),
                ),
                grant(
                    Object::Node(ENG),
                    Relation::Viewer,
                    Subject::User(MARCUS),
                ),
                grant(
                    Object::Node(ENG),
                    Relation::Manager,
                    Subject::User(LENA),
                ),
                grant(
                    Object::Node(SPEC),
                    Relation::Editor,
                    Subject::User(MARCUS),
                ),
                grant(
                    Object::Node(SPEC),
                    Relation::Editor,
                    Subject::User(Id(9)),
                ),
                Change::Restrict(SPEC),
            ],
        )
        .await
        .unwrap();
    assert!(!can(&authz, user(PRIYA), SPEC, Permission::Read).await);
    let marcus = authz
        .check(SPACE, user(MARCUS), SPEC, Permission::Read)
        .await
        .unwrap();
    assert_eq!(marcus.relation, Some(Relation::Viewer));
    assert!(
        !can(&authz, user(Id(9)), SPEC, Permission::Read).await,
        "narrowing never adds"
    );
    assert!(can(&authz, user(LENA), SPEC, Permission::Manage).await);

    let _ = authz
        .write(SPACE, Actor::User(LENA), &[Change::Unrestrict(SPEC)])
        .await
        .unwrap();
    assert!(can(&authz, user(PRIYA), SPEC, Permission::Edit).await);
    assert!(can(&authz, user(Id(9)), SPEC, Permission::Edit).await);
}

#[tokio::test]
async fn expired_grants_are_swept() {
    let now = Arc::new(AtomicI64::new(1_000));
    let clock = Arc::clone(&now);
    let store = MemoryStore::new();
    store.set_tree(SPACE, tree()).unwrap();
    let authz = Authz::with_clock(
        store,
        Arc::new(move || clock.load(Ordering::SeqCst)),
    );
    let t =
        Tuple::role(Object::Node(SPEC), Relation::Viewer, Subject::User(PRIYA));
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[
                Change::Grant(t.expiring(2_000)),
                Change::Grant(
                    Tuple::role(
                        Object::Node(ENG),
                        Relation::Viewer,
                        Subject::User(LENA),
                    )
                    .expiring(5_000),
                ),
            ],
        )
        .await
        .unwrap();
    assert!(can(&authz, user(PRIYA), SPEC, Permission::Read).await);
    assert_eq!(authz.sweep_expired(SPACE).await.unwrap(), Some(2_000));

    now.store(2_000, Ordering::SeqCst);
    assert_eq!(authz.sweep_expired(SPACE).await.unwrap(), Some(5_000));
    assert!(!can(&authz, user(PRIYA), SPEC, Permission::Read).await);
    assert!(can(&authz, user(LENA), SPEC, Permission::Read).await);
    let graph = authz.store().load_space(SPACE).await.unwrap();
    assert_eq!(graph.tuples.len(), 1, "expired tuple deleted");
}

#[tokio::test]
async fn filter_is_an_exact_batch_check() {
    let authz = setup().await;
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Node(RUNBOOKS),
                Relation::Viewer,
                Subject::User(PRIYA),
            )],
        )
        .await
        .unwrap();
    let asked = [SPEC, ONCALL, RUNBOOKS, Id(0xbeef)];
    assert_eq!(
        authz
            .filter(user(PRIYA), Permission::Read, &asked)
            .await
            .unwrap(),
        BTreeSet::from([ONCALL, RUNBOOKS])
    );
    assert!(
        authz
            .filter(user(PRIYA), Permission::Edit, &asked)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn bad_changes_are_rejected() {
    let authz = setup().await;
    let cases = [
        grant(
            Object::Node(Id(0xdead)),
            Relation::Viewer,
            Subject::User(PRIYA),
        ),
        grant(
            Object::Space(OTHER_SPACE),
            Relation::Viewer,
            Subject::User(PRIYA),
        ),
        grant(
            Object::Group(PLATFORM),
            Relation::Viewer,
            Subject::User(PRIYA),
        ),
        Change::Grant(Tuple::member(PLATFORM, Subject::User(PRIYA))),
        Change::Revoke {
            object: Object::Group(PLATFORM),
            relation: Relation::Viewer,
            subject: Subject::User(PRIYA),
        },
    ];
    for change in cases {
        let err = authz
            .write(SPACE, Actor::System, &[change])
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::UnknownNode { .. }
                    | Error::WrongSpace { .. }
                    | Error::Malformed(_)
            ),
            "{change:?} gave {err:?}"
        );
    }
    assert!(matches!(
        authz.epoch(OTHER_SPACE).await,
        Err(Error::Store(StoreError::UnknownSpace(_)))
    ));
}

#[tokio::test]
async fn stale_commits_conflict() {
    let store = MemoryStore::new();
    store.set_tree(SPACE, tree()).unwrap();
    let commit = |expected_epoch| Commit {
        space: SPACE,
        expected_epoch,
        upsert: vec![],
        delete: vec![],
        scope: Scope::All,
        rows: vec![],
    };
    assert_eq!(store.commit(commit(0)).await.unwrap(), 1);
    assert!(matches!(
        store.commit(commit(0)).await,
        Err(StoreError::Conflict)
    ));
}

#[tokio::test]
async fn permission_changes_are_visible_within_a_second() {
    // 2,000 nodes, 20 users with folder grants: one write re-materialises
    // the space and the next check sees it.
    let store = MemoryStore::new();
    let mut parents = BTreeMap::new();
    for i in 0..2_000_u128 {
        let _ = parents.insert(
            Id(10_000 + i),
            if i < 20 {
                None
            } else {
                Some(Id(10_000 + i % 20))
            },
        );
    }
    store.set_tree(SPACE, parents).unwrap();
    let authz = Authz::new(store);
    let changes: Vec<Change> = (0..20_u128)
        .map(|i| {
            grant(
                Object::Node(Id(10_000 + i)),
                Relation::Editor,
                Subject::User(Id(i + 1)),
            )
        })
        .collect();
    let _ = authz.write(SPACE, Actor::System, &changes).await.unwrap();

    let started = Instant::now();
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[grant(
                Object::Space(SPACE),
                Relation::Viewer,
                Subject::User(Id(500)),
            )],
        )
        .await
        .unwrap();
    assert!(can(&authz, user(Id(500)), Id(11_999), Permission::Read).await);
    let elapsed = started.elapsed();
    assert!(elapsed.as_millis() < 1_000, "took {elapsed:?}");
}
