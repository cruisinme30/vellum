// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: materialisation against an independent reference, and
//! the inheritance rules as invariants.

// Test helpers outside #[test] functions may unwrap like the tests do.
#![allow(
    clippy::unwrap_used,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else
)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use vellum_authz::{
    Actor, Authz, Change, Directory, Id, MemoryStore, Object, Permission,
    Principal, Relation, SpaceGraph, Subject, Tuple, TupleRelation,
    materialise,
};

const SPACE: Id = Id(1 << 64);
const NOW: i64 = 1_000;
const USERS: u128 = 4;
const GROUPS: u128 = 2;

fn node_id(i: usize) -> Id {
    Id(100 + i as u128)
}

fn relation() -> impl Strategy<Value = Relation> {
    prop::sample::select(Relation::ALL.to_vec())
}

fn subject() -> impl Strategy<Value = Subject> {
    prop_oneof![
        (1..=USERS).prop_map(|u| Subject::User(Id(u))),
        (0..GROUPS).prop_map(|g| Subject::Group(Id(50 + g))),
        Just(Subject::ShareLink(Id(70))),
    ]
}

/// A random forest: node i's parent is an earlier node or the space.
fn forest(max: usize) -> impl Strategy<Value = BTreeMap<Id, Option<Id>>> {
    (1..max).prop_flat_map(|n| {
        prop::collection::vec(
            prop::option::weighted(0.7, any::<prop::sample::Index>()),
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
                        pick.map(|ix| node_id(ix.index(i)))
                    };
                    (node_id(i), parent)
                })
                .collect()
        })
    })
}

#[derive(Debug, Clone)]
struct World {
    graph: SpaceGraph,
    members: Vec<Tuple>,
}

fn world() -> impl Strategy<Value = World> {
    forest(12).prop_flat_map(|parents| {
        let n = parents.len();
        let object = prop_oneof![
            1 => Just(Object::Space(SPACE)),
            4 => (0..n).prop_map(|i| Object::Node(node_id(i))),
        ];
        let expiry = prop::option::weighted(0.15, 0_i64..2_000);
        let role =
            (object, relation(), subject(), expiry).prop_map(|(o, r, s, e)| {
                Tuple {
                    expires_at: e,
                    ..Tuple::role(o, r, s)
                }
            });
        let restricted = (0..n).prop_map(|i| Tuple::restricted(node_id(i)));
        let member = (
            0..GROUPS,
            prop_oneof![
                3 => (1..=USERS).prop_map(|u| Subject::User(Id(u))),
                1 => (0..GROUPS).prop_map(|g| Subject::Group(Id(50 + g))),
            ],
            prop::option::weighted(0.25, 0_i64..2_500),
        )
            .prop_map(|(g, s, e)| Tuple {
                expires_at: e,
                ..Tuple::member(Id(50 + g), s)
            });
        (
            Just(parents),
            prop::collection::vec(role, 0..16),
            prop::collection::vec(restricted, 0..3),
            prop::collection::vec(member, 0..5),
        )
            .prop_map(|(parents, mut tuples, restricted, members)| {
                tuples.extend(restricted);
                World {
                    graph: SpaceGraph {
                        space: SPACE,
                        epoch: 0,
                        parents,
                        tuples,
                    },
                    members,
                }
            })
    })
}

fn principals() -> Vec<Principal> {
    let mut out: Vec<Principal> =
        (1..=USERS + 1).map(|u| Principal::User(Id(u))).collect();
    out.push(Principal::ShareLink(Id(70)));
    out
}

/// Whether `p` is in `group` at time `t`, by plain reachability over the
/// memberships live at `t` (independent of `Directory`).
fn in_group(members: &[Tuple], group: Id, p: Principal, t: i64) -> bool {
    let mut seen = vec![group];
    let mut stack = vec![group];
    while let Some(g) = stack.pop() {
        for m in members
            .iter()
            .filter(|m| m.object == Object::Group(g) && m.is_live(t))
        {
            match m.subject {
                Subject::Group(inner) if !seen.contains(&inner) => {
                    seen.push(inner);
                    stack.push(inner);
                }
                other if other.principal() == Some(p) => return true,
                _ => {}
            }
        }
    }
    false
}

/// The rules written as a walk up the tree at one instant, independent of
/// the top-down, time-ranged materialisation.
fn reference(
    g: &SpaceGraph,
    members: &[Tuple],
    p: Principal,
    node: Id,
    t: i64,
) -> Option<Relation> {
    let grant_on = |object: Object| -> Option<Relation> {
        g.tuples
            .iter()
            .filter(|tu| tu.object == object && tu.is_live(t))
            .filter_map(|tu| match tu.relation {
                TupleRelation::Role(r) => Some((tu.subject, r)),
                _ => None,
            })
            .filter(|(s, _)| match s {
                Subject::Group(id) => in_group(members, *id, p, t),
                other => other.principal() == Some(p),
            })
            .map(|(_, r)| r)
            .max()
    };
    let restricted = |n: Id| {
        g.tuples.iter().any(|tu| {
            tu.object == Object::Node(n)
                && tu.relation == TupleRelation::Restricted
        })
    };
    let mut local = None;
    let mut cur = node;
    loop {
        local = local.max(grant_on(Object::Node(cur)));
        let parent = g.parents.get(&cur).copied().flatten();
        if restricted(cur) {
            let above = match parent {
                Some(par) => reference(g, members, p, par, t),
                None => grant_on(Object::Space(SPACE)),
            };
            return match above {
                Some(a) if a >= Relation::Manager => Some(a),
                Some(a) => local.map(|l| l.min(a)),
                None => None,
            };
        }
        match parent {
            Some(par) => cur = par,
            None => return local.max(grant_on(Object::Space(SPACE))),
        }
    }
}

fn access_of(w: &World) -> BTreeMap<(Principal, Id), Relation> {
    let dir = Directory::new(&w.members, NOW);
    let m = materialise(&w.graph, &dir, NOW).unwrap();
    m.nodes
        .iter()
        .flat_map(|(n, a)| {
            a.iter()
                .filter_map(move |(p, l)| Some(((*p, *n), l.at(NOW)?)))
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn materialise_matches_reference(w in world()) {
        let dir = Directory::new(&w.members, NOW);
        let m = materialise(&w.graph, &dir, NOW).unwrap();
        // Rows computed once at NOW must stay exact as grants and
        // memberships expire later, with no sweep in between.
        for t in [NOW, 1_200, 1_700, 2_100, 3_000] {
            for p in principals() {
                for &node in w.graph.parents.keys() {
                    prop_assert_eq!(
                        m.nodes[&node].get(&p).and_then(|l| l.at(t)),
                        reference(&w.graph, &w.members, p, node, t),
                        "principal {:?} node {} at {}", p, node, t
                    );
                }
            }
        }
    }

    #[test]
    fn deny_by_default(w in world()) {
        // User USERS+1 appears in no tuple.
        let outsider = Principal::User(Id(USERS + 1));
        prop_assert!(access_of(&w).keys().all(|(p, _)| *p != outsider));
    }

    #[test]
    fn grants_only_add(
        w in world(),
        extra in (relation(), subject()),
        at in any::<prop::sample::Index>()
    ) {
        let before = access_of(&w);
        let mut more = w.clone();
        let nodes: Vec<Id> = w.graph.parents.keys().copied().collect();
        let target = nodes[at.index(nodes.len())];
        more.graph.tuples.push(
            Tuple::role(Object::Node(target), extra.0, extra.1)
        );
        let after = access_of(&more);
        for (key, r) in before {
            prop_assert!(after.get(&key).is_some_and(|a| *a >= r), "{:?} lost \
                access", key);
        }
    }

    #[test]
    fn restrictions_only_remove(
        w in world(),
        at in any::<prop::sample::Index>()
    ) {
        let before = access_of(&w);
        let mut narrowed = w.clone();
        let nodes: Vec<Id> = w.graph.parents.keys().copied().collect();
        narrowed.graph.tuples.push(
            Tuple::restricted(nodes[at.index(nodes.len())])
        );
        for (key, r) in access_of(&narrowed) {
            prop_assert!(
                before.get(&key).is_some_and(|b| *b >= r),
                "{:?} gained access",
                key
            );
        }
    }

    #[test]
    fn moved_node_keeps_explicit_and_drops_inherited(
        w in world(),
        at in any::<prop::sample::Index>()
    ) {
        // Move a node to the top of a space with no space-level grants and
        // no restrictions: its access is exactly its own explicit grants.
        let mut moved = w.clone();
        moved.graph.tuples.retain(|t| {
            t.relation != TupleRelation::Restricted && t.object != Object::Space(
                SPACE
            )
        });
        let nodes: Vec<Id> = w.graph.parents.keys().copied().collect();
        let node = nodes[at.index(nodes.len())];
        let _ = moved.graph.parents.insert(node, None);
        let dir = Directory::new(&moved.members, NOW);
        let m = materialise(&moved.graph, &dir, NOW).unwrap();
        for p in principals() {
            let explicit = moved.graph.tuples.iter()
                .filter(|t| t.object == Object::Node(node) && t.is_live(NOW))
                .filter_map(|t| match (t.relation, t.subject) {
                    (
                        TupleRelation::Role(r),
                        Subject::Group(g)
                    ) => in_group(&moved.members, g, p, NOW).then_some(r),
                    (
                        TupleRelation::Role(r),
                        s
                    ) => (s.principal() == Some(p)).then_some(r),
                    _ => None,
                })
                .max();
            prop_assert_eq!(
                m.nodes[&node].get(&p).and_then(|l| l.at(NOW)),
                explicit
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Incremental subtree writes through the service end in the same rows
    /// as one full rebuild.
    #[test]
    fn incremental_writes_equal_a_rebuild(
        w in world(),
        order in prop::collection::vec(any::<prop::sample::Index>(), 1..6)
    ) {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let store = MemoryStore::new();
            store.set_tree(SPACE, w.graph.parents.clone()).unwrap();
            let authz = Authz::with_clock(store, std::sync::Arc::new(|| NOW));
            vellum_authz::AuthzStore::write_directory(
                authz.store(),
                w.members.clone(),
                vec![]
            ).await.unwrap();
            let live: Vec<Tuple> = w.graph.tuples.iter().copied().filter(
                |t| t.is_live(NOW)
            ).collect();
            for (i, t) in live.iter().enumerate() {
                let change = match t.relation {
                    TupleRelation::Restricted => match t.object {
                        Object::Node(n) => Change::Restrict(n),
                        _ => unreachable!(),
                    },
                    _ => Change::Grant(*t),
                };
                let _ = authz.write(
                    SPACE,
                    Actor::System,
                    &[change]
                ).await.unwrap();
                // Interleave some revocations of earlier grants.
                if let Some(ix) = order.get(i)
                    && let Some(old) = live.get(ix.index(i + 1))
                    && let TupleRelation::Role(r) = old.relation
                    && ix.index(4) == 0
                {
                    let _ = authz.write(
                        SPACE,
                        Actor::System,
                        &[Change::Revoke {
                            object: old.object,
                            relation: r,
                            subject: old.subject
                        }]
                    ).await.unwrap();
                }
            }
            let incremental = snapshot(authz.store()).await;
            let _ = authz.rebuild(SPACE).await.unwrap();
            let rebuilt = snapshot(authz.store()).await;
            prop_assert_eq!(&incremental, &rebuilt);
            // And checks agree with the rows.
            for p in principals() {
                for &n in w.graph.parents.keys() {
                    let d = authz.check(
                        SPACE,
                        p,
                        n,
                        Permission::Read
                    ).await.unwrap();
                    prop_assert_eq!(
                        d.relation,
                        rebuilt.get(&(NOW, p, n)).copied()
                    );
                }
            }
            Ok(())
        })?;
    }
}

/// The strongest live relation per (time, principal, object), read back
/// through the store at a few instants.
async fn snapshot(
    store: &MemoryStore,
) -> BTreeMap<(i64, Principal, Id), Relation> {
    let mut out = BTreeMap::new();
    for t in [NOW, 1_700, 2_100] {
        for p in principals() {
            for (o, r) in vellum_authz::AuthzStore::list(store, SPACE, p, t)
                .await
                .unwrap()
            {
                let slot = out.entry((t, p, o)).or_insert(r);
                *slot = (*slot).max(r);
            }
        }
    }
    out
}
