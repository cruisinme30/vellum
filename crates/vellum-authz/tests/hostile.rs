// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red-team regressions: expiry must fail closed at check time, with no
//! sweep job running (#267, #269).

// Test helpers outside #[test] functions may unwrap like the tests do.
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use vellum_authz::{
    Actor, Authz, AuthzStore as _, Change, Id, MemoryStore, Object, Permission, Principal,
    Relation, Subject, Tuple,
};

const SPACE: Id = Id(0x5000);
const DOC: Id = Id(0x200);
const GROUP: Id = Id(0x900);
const OUTER: Id = Id(0x901);
const USER: Id = Id(1);
const LINK: Id = Id(7);

fn setup() -> (Authz<MemoryStore>, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000));
    let clock = Arc::clone(&now);
    let store = MemoryStore::new();
    store
        .set_tree(SPACE, BTreeMap::from([(DOC, None)]))
        .unwrap();
    let authz = Authz::with_clock(store, Arc::new(move || clock.load(Ordering::SeqCst)));
    (authz, now)
}

async fn relation(authz: &Authz<MemoryStore>, who: Principal) -> Option<Relation> {
    authz
        .check(SPACE, who, DOC, Permission::Read)
        .await
        .unwrap()
        .relation
}

const fn grant(relation: Relation, subject: Subject) -> Tuple {
    Tuple::role(Object::Node(DOC), relation, subject)
}

#[tokio::test]
async fn expired_group_membership_stops_granting_without_a_sweep() {
    let (authz, now) = setup();
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[Change::Grant(grant(
                Relation::Editor,
                Subject::Group(GROUP),
            ))],
        )
        .await
        .unwrap();
    authz
        .store()
        .write_directory(
            vec![Tuple::member(GROUP, Subject::User(USER)).expiring(2_000)],
            vec![],
        )
        .await
        .unwrap();
    let _ = authz.rebuild(SPACE).await.unwrap();
    let user = Principal::User(USER);
    assert_eq!(relation(&authz, user).await, Some(Relation::Editor));

    now.store(10_000, Ordering::SeqCst);
    assert_eq!(relation(&authz, user).await, None, "no sweep has run");
    assert!(
        authz
            .accessible(SPACE, user, Permission::Read)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        authz
            .filter(user, Permission::Read, &[DOC])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(authz.spaces(user).await.unwrap().is_empty());

    // The sweep then deletes the dead membership too.
    let _ = authz.sweep_expired(SPACE).await.unwrap();
    assert!(authz.store().load_directory().await.unwrap().is_empty());
    assert_eq!(relation(&authz, user).await, None);
}

#[tokio::test]
async fn nested_membership_expires_with_its_weakest_link() {
    let (authz, now) = setup();
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[Change::Grant(grant(
                Relation::Viewer,
                Subject::Group(OUTER),
            ))],
        )
        .await
        .unwrap();
    authz
        .store()
        .write_directory(
            vec![
                Tuple::member(OUTER, Subject::Group(GROUP)).expiring(3_000),
                Tuple::member(GROUP, Subject::User(USER)).expiring(5_000),
            ],
            vec![],
        )
        .await
        .unwrap();
    let _ = authz.rebuild(SPACE).await.unwrap();
    let user = Principal::User(USER);
    now.store(2_999, Ordering::SeqCst);
    assert_eq!(relation(&authz, user).await, Some(Relation::Viewer));
    now.store(3_000, Ordering::SeqCst);
    assert_eq!(relation(&authz, user).await, None);
}

#[tokio::test]
async fn expired_share_link_fails_closed_without_a_sweep() {
    let (authz, now) = setup();
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[Change::Grant(
                grant(Relation::Viewer, Subject::ShareLink(LINK)).expiring(2_000),
            )],
        )
        .await
        .unwrap();
    let link = Principal::ShareLink(LINK);
    assert_eq!(relation(&authz, link).await, Some(Relation::Viewer));
    now.store(1_000_000, Ordering::SeqCst);
    assert_eq!(relation(&authz, link).await, None);
}

#[tokio::test]
async fn a_weaker_lasting_grant_survives_a_stronger_expiring_one() {
    let (authz, now) = setup();
    let _ = authz
        .write(
            SPACE,
            Actor::System,
            &[
                Change::Grant(grant(Relation::Manager, Subject::User(USER)).expiring(2_000)),
                Change::Grant(Tuple::role(
                    Object::Space(SPACE),
                    Relation::Viewer,
                    Subject::User(USER),
                )),
            ],
        )
        .await
        .unwrap();
    let user = Principal::User(USER);
    assert_eq!(relation(&authz, user).await, Some(Relation::Manager));
    // An expired manager can no longer share.
    now.store(2_000, Ordering::SeqCst);
    assert_eq!(relation(&authz, user).await, Some(Relation::Viewer));
    let share = [Change::Grant(grant(Relation::Viewer, Subject::User(Id(9))))];
    assert!(authz.write(SPACE, Actor::User(USER), &share).await.is_err());
    assert_eq!(
        authz
            .filter(user, Permission::Read, &[DOC, SPACE])
            .await
            .unwrap(),
        BTreeSet::from([DOC, SPACE])
    );
    assert!(
        authz
            .filter(user, Permission::Manage, &[DOC])
            .await
            .unwrap()
            .is_empty()
    );
}
