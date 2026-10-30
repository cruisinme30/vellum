// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The SQL adapter against the in-memory reference: the same random writes
//! go to both, and every check, list, batch filter and space listing must
//! agree, now and after grants expire. SQLite runs in a temporary file;
//! Postgres runs in a throwaway container.

// Test helpers outside #[test] functions may unwrap like the tests do, and
// run on a current-thread runtime, so their futures need not be Send.
#![allow(
    clippy::unwrap_used,
    clippy::missing_const_for_fn,
    clippy::future_not_send
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_authz::{
    Actor, Authz, AuthzStore, Change, Id, MemoryStore, Object, Permission,
    Principal, Relation, SqlStore, StoreError, Subject, Tuple,
};
use vellum_db::sqlx::types::Uuid;
use vellum_db::{Index, IndexConfig, IndexPool, sqlx};

const SPACE: Id = Id(0x0193_0000_0000_7000_8000_0000_0000_0001);
const NOW: i64 = 1_000;
const TIMES: [i64; 4] = [NOW, 1_500, 2_100, 9_000];
const USERS: u128 = 3;
const GROUP: Id = Id(50);

fn uuid(id: Id) -> Uuid {
    Uuid::from_u128(id.0)
}

fn node(i: usize) -> Id {
    Id(100 + i as u128)
}

/// Writes the space row and its `page_ancestors` closure, as a reindex does
/// (the space is the root of every chain).
async fn seed(index: &Index, parents: &BTreeMap<Id, Option<Id>>) {
    let mut rows = vec![(SPACE, SPACE, 0_i32)];
    for &n in parents.keys() {
        let mut chain = vec![n];
        let mut cur = n;
        while let Some(Some(p)) = parents.get(&cur) {
            chain.push(*p);
            cur = *p;
        }
        chain.push(SPACE);
        for (depth, ancestor) in chain.iter().enumerate() {
            rows.push((*ancestor, n, i32::try_from(depth).unwrap()));
        }
    }
    macro_rules! write {
        ($pool:expr) => {{
            let mut tx = $pool.begin().await.unwrap();
            let _ = sqlx::query(
                "INSERT INTO spaces (space_id, kind, slug, root_path, name, \
                 indexed_at) VALUES ($1, 'shared_drive', 'eng', \
                 'drives/shared/eng', 'Engineering', 0) ON CONFLICT (space_id) \
                 DO NOTHING",
            )
            .bind(uuid(SPACE))
            .execute(&mut *tx)
            .await
            .unwrap();
            let _ = sqlx::query(
                "DELETE FROM page_ancestors WHERE space_id = $1"
            )
                .bind(uuid(SPACE))
                .execute(&mut *tx)
                .await
                .unwrap();
            for (ancestor, descendant, depth) in &rows {
                let _ = sqlx::query(
                    "INSERT INTO page_ancestors (space_id, ancestor_id, \
                     descendant_id, depth) VALUES ($1, $2, $3, $4)",
                )
                .bind(uuid(SPACE))
                .bind(uuid(*ancestor))
                .bind(uuid(*descendant))
                .bind(*depth)
                .execute(&mut *tx)
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
        }};
    }
    match index.pool() {
        IndexPool::Sqlite(pool) => write!(pool),
        IndexPool::Postgres(pool) => write!(pool),
        _ => unreachable!(),
    }
}

async fn reset(index: &Index) {
    const RESET: [&str; 4] = [
        "DELETE FROM effective_access",
        "DELETE FROM grants",
        "DELETE FROM page_ancestors",
        "DELETE FROM spaces",
    ];
    for sql in RESET {
        match index.pool() {
            IndexPool::Sqlite(pool) => {
                let _ = sqlx::query(sql).execute(pool).await.unwrap();
            }
            IndexPool::Postgres(pool) => {
                let _ = sqlx::query(sql).execute(pool).await.unwrap();
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone)]
struct Case {
    parents: BTreeMap<Id, Option<Id>>,
    moved: Option<(usize, Option<usize>)>,
    changes: Vec<Change>,
    members: Vec<Tuple>,
}

fn subject() -> impl Strategy<Value = Subject> {
    prop_oneof![
        3 => (1..=USERS).prop_map(|u| Subject::User(Id(u))),
        1 => Just(Subject::Group(GROUP)),
        1 => Just(Subject::ShareLink(Id(70))),
    ]
}

fn case() -> impl Strategy<Value = Case> {
    (2_usize..8).prop_flat_map(|n| {
        let parents = prop::collection::vec(
            prop::option::of(any::<prop::sample::Index>()),
            n,
        )
        .prop_map(|picks| {
            picks
                .into_iter()
                .enumerate()
                .map(|(i, pick)| {
                    let parent = if i == 0 {
                        None
                    } else {
                        pick.map(|ix| node(ix.index(i)))
                    };
                    (node(i), parent)
                })
                .collect::<BTreeMap<_, _>>()
        });
        let object = prop_oneof![
            1 => Just(Object::Space(SPACE)),
            4 => (0..n).prop_map(|i| Object::Node(node(i))),
        ];
        let grant = (
            object,
            prop::sample::select(Relation::ALL.to_vec()),
            subject(),
            prop::option::weighted(0.3, 1_001_i64..3_000),
        )
            .prop_map(|(o, r, s, e)| {
                Change::Grant(Tuple {
                    expires_at: e,
                    ..Tuple::role(o, r, s)
                })
            });
        let restrict = (0..n).prop_map(|i| Change::Restrict(node(i)));
        let change = prop_oneof![5 => grant, 1 => restrict];
        let member = (1..=USERS, prop::option::weighted(0.3, 1_001_i64..3_000))
            .prop_map(|(u, e)| Tuple {
                expires_at: e,
                ..Tuple::member(GROUP, Subject::User(Id(u)))
            });
        (
            parents,
            prop::option::of((0..n, prop::option::of(0..n))),
            prop::collection::vec(change, 1..10),
            prop::collection::vec(member, 0..3),
        )
            .prop_map(|(parents, moved, changes, members)| Case {
                parents,
                moved,
                changes,
                members,
            })
    })
}

fn principals() -> Vec<Principal> {
    let mut out: Vec<Principal> =
        (1..=USERS + 1).map(|u| Principal::User(Id(u))).collect();
    out.push(Principal::ShareLink(Id(70)));
    out
}

/// A move that keeps the tree a forest, or none.
fn moved_tree(case: &Case) -> Option<BTreeMap<Id, Option<Id>>> {
    let (child, parent) = case.moved?;
    let child = node(child);
    let parent = parent.map(node);
    let mut cur = parent;
    while let Some(p) = cur {
        if p == child {
            return None;
        }
        cur = case.parents.get(&p).copied().flatten();
    }
    let mut tree = case.parents.clone();
    let _ = tree.insert(child, parent);
    Some(tree)
}

/// Runs the case against both stores and compares every answer.
async fn run(index: &Index, case: &Case) -> Result<(), TestCaseError> {
    reset(index).await;
    let clock = Arc::new(AtomicI64::new(NOW));
    let now = || {
        let clock = Arc::clone(&clock);
        Arc::new(move || clock.load(Ordering::SeqCst))
    };
    let memory = MemoryStore::new();
    memory.set_tree(SPACE, case.parents.clone()).unwrap();
    seed(index, &case.parents).await;
    let reference = Authz::with_clock(memory, now());
    let sql = Authz::with_clock(SqlStore::new(index.clone()), now());

    for store in [
        reference.store() as &dyn DirWriter,
        sql.store() as &dyn DirWriter,
    ] {
        store.add_members(case.members.clone()).await;
    }
    for change in &case.changes {
        let a = reference
            .write(SPACE, Actor::System, &[*change])
            .await
            .is_ok();
        let b = sql.write(SPACE, Actor::System, &[*change]).await.is_ok();
        prop_assert_eq!(a, b, "write {:?}", change);
    }
    if let Some(tree) = moved_tree(case) {
        let child = node(case.moved.unwrap().0);
        reference.store().set_tree(SPACE, tree.clone()).unwrap();
        seed(index, &tree).await;
        let _ = reference.tree_changed(SPACE, &[child]).await.unwrap();
        let _ = sql.tree_changed(SPACE, &[child]).await.unwrap();
    }
    prop_assert_eq!(
        reference.epoch(SPACE).await.unwrap(),
        sql.epoch(SPACE).await.unwrap()
    );

    let mut objects: Vec<Id> = case.parents.keys().copied().collect();
    objects.push(SPACE);
    for t in TIMES {
        clock.store(t, Ordering::SeqCst);
        for p in principals() {
            for &o in &objects {
                let a = reference
                    .check(SPACE, p, o, Permission::Read)
                    .await
                    .unwrap();
                let b = sql.check(SPACE, p, o, Permission::Read).await.unwrap();
                prop_assert_eq!(
                    a.relation,
                    b.relation,
                    "{:?} on {} at {}",
                    p,
                    o,
                    t
                );
            }
            for perm in [Permission::Read, Permission::Edit, Permission::Manage]
            {
                let mut a = reference.accessible(SPACE, p, perm).await.unwrap();
                let mut b = sql.accessible(SPACE, p, perm).await.unwrap();
                a.sort();
                b.sort();
                prop_assert_eq!(a, b);
                prop_assert_eq!(
                    reference.filter(p, perm, &objects).await.unwrap(),
                    sql.filter(p, perm, &objects).await.unwrap()
                );
            }
            prop_assert_eq!(
                reference.spaces(p).await.unwrap(),
                sql.spaces(p).await.unwrap()
            );
        }
    }
    // The sweep collects the same garbage on both.
    clock.store(9_000, Ordering::SeqCst);
    let _ = reference.sweep_expired(SPACE).await.unwrap();
    let _ = sql.sweep_expired(SPACE).await.unwrap();
    let tuples = |g: vellum_authz::SpaceGraph| {
        g.tuples.into_iter().collect::<BTreeSet<_>>()
    };
    prop_assert_eq!(
        tuples(reference.store().load_space(SPACE).await.unwrap()),
        tuples(sql.store().load_space(SPACE).await.unwrap())
    );
    prop_assert_eq!(
        reference.store().load_directory().await.unwrap().len(),
        sql.store().load_directory().await.unwrap().len()
    );
    Ok(())
}

/// Membership writes through either store, behind one object-safe call.
trait DirWriter {
    fn add_members(
        &self,
        members: Vec<Tuple>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>>;
}

impl<S: AuthzStore> DirWriter for S {
    fn add_members(
        &self,
        members: Vec<Tuple>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + '_>> {
        Box::pin(
            async move { self.write_directory(members, vec![]).await.unwrap() },
        )
    }
}

fn differential(index: &Index, rt: &tokio::runtime::Runtime, cases: u32) {
    let mut runner = TestRunner::new(Config {
        cases,
        ..Config::default()
    });
    runner
        .run(&case(), |c| rt.block_on(run(index, &c)))
        .unwrap();
}

#[test]
fn sqlite_matches_the_reference() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let index = rt
        .block_on(Index::open(&IndexConfig::sqlite(
            dir.path().join("index.sqlite"),
            None,
        )))
        .unwrap();
    differential(&index, &rt, 48);
}

#[test]
fn postgres_matches_the_reference() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    // Keeps a runtime in scope if a failing case unwinds past the container.
    let _context = rt.enter();
    let (container, index): (ContainerAsync<Postgres>, Index) =
        rt.block_on(async {
            let container = Postgres::default()
                .with_tag("17-alpine")
                .start()
                .await
                .unwrap();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let url = format!(
                "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
            );
            let index = Index::open(&IndexConfig::postgres(url)).await.unwrap();
            (container, index)
        });
    differential(&index, &rt, 24);
    // The container must be removed inside the runtime: dropping it outside
    // one panics.
    rt.block_on(async move {
        drop(index);
        container.rm().await.unwrap();
    });
}

#[tokio::test]
async fn unknown_spaces_and_stale_epochs_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(&IndexConfig::sqlite(
        dir.path().join("index.sqlite"),
        None,
    ))
    .await
    .unwrap();
    let store = SqlStore::new(index.clone());
    assert!(matches!(
        store.load_space(SPACE).await,
        Err(StoreError::UnknownSpace(_))
    ));
    assert!(matches!(
        store.epoch(SPACE).await,
        Err(StoreError::UnknownSpace(_))
    ));
    seed(&index, &BTreeMap::from([(node(0), None)])).await;
    let commit = |expected_epoch| vellum_authz::Commit {
        space: SPACE,
        expected_epoch,
        upsert: vec![],
        delete: vec![],
        scope: vellum_authz::Scope::All,
        rows: vec![],
    };
    assert_eq!(store.commit(commit(0)).await.unwrap(), 1);
    assert!(matches!(
        store.commit(commit(0)).await,
        Err(StoreError::Conflict)
    ));
    let graph = store.load_space(SPACE).await.unwrap();
    assert_eq!(graph.parents, BTreeMap::from([(node(0), None)]));
    assert_eq!(graph.epoch, 1);
}
