// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Materialising effective access from tuples and the folder tree.
//!
//! The rules (ADR 0008):
//!
//! - **Deny by default.** A principal has access to a node only through a
//!   live tuple on the node, an ancestor or the space, directly or through
//!   group membership.
//! - **Inheritance.** A node's access is its parent's access merged with
//!   its own grants; the strongest relation wins. Top-level nodes inherit
//!   from the space.
//! - **Narrowing.** A node marked `restricted` (a book section, R-BK-9)
//!   keeps only those inherited principals it grants explicitly, each
//!   capped at what they had above it, plus anyone who is a manager or
//!   owner above it. Grants further down are capped the same way, so a
//!   restriction can only remove access, never add it.
//! - **Moves.** Grants are tuples on the node, so a moved node keeps them;
//!   inherited access is recomputed from the new parent (R-DR-2).
//! - **Shortcuts** are ordinary nodes. Nothing here follows a shortcut to
//!   its target, so a shortcut never grants access to it (R-DR-3).
//! - **Expiry** is exact without a sweep. Each principal's access is a
//!   [`Levels`]: for every relation, the time until which they hold at
//!   least that relation, taking the best derivation through grants and
//!   group memberships and the earliest expiry along each. Rows carry that
//!   time and checks ignore rows past it, so access fails closed the
//!   moment a grant or membership expires (D54).

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    Id, Object, Principal, Relation, Subject, Tuple, TupleRelation,
};

/// "Never expires" as a validity time.
pub const FOREVER: i64 = i64::MAX;
/// "Never held" as a validity time.
const NEVER: i64 = i64::MIN;

const fn until(expires_at: Option<i64>) -> i64 {
    match expires_at {
        Some(at) => at,
        None => FOREVER,
    }
}

/// One principal's access on one object over time.
///
/// For each relation, the UTC epoch millisecond until which (exclusive)
/// they hold at least that relation. Weaker relations are always held at
/// least as long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels([i64; 5]);

impl Default for Levels {
    fn default() -> Self {
        Self([NEVER; 5])
    }
}

const fn level(r: Relation) -> usize {
    r as usize
}

impl Levels {
    /// Holding `relation` (and everything below it) until `until`.
    #[must_use]
    pub fn granting(relation: Relation, until: i64) -> Self {
        let mut out = Self::default();
        for (i, slot) in out.0.iter_mut().enumerate() {
            if i <= level(relation) {
                *slot = until;
            }
        }
        out
    }

    /// Until when the principal holds at least `relation`.
    #[must_use]
    pub const fn until(&self, relation: Relation) -> i64 {
        let [viewer, commenter, editor, manager, owner] = self.0;
        match relation {
            Relation::Viewer => viewer,
            Relation::Commenter => commenter,
            Relation::Editor => editor,
            Relation::Manager => manager,
            Relation::Owner => owner,
        }
    }

    /// The strongest relation held at `now_ms`, if any.
    #[must_use]
    pub fn at(&self, now_ms: i64) -> Option<Relation> {
        Relation::ALL
            .into_iter()
            .rev()
            .find(|r| self.until(*r) > now_ms)
    }

    /// Whether nothing is held after `now_ms`.
    #[must_use]
    pub fn is_dead(&self, now_ms: i64) -> bool {
        self.at(now_ms).is_none()
    }

    /// Either derivation: the later time at each level.
    #[must_use]
    pub fn or(mut self, other: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a = (*a).max(b);
        }
        self
    }

    /// Both must hold: the earlier time at each level.
    #[must_use]
    pub fn and(mut self, other: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a = (*a).min(b);
        }
        self
    }

    /// The part of `self` held while it is at least a manager: what a
    /// restriction cannot take away.
    #[must_use]
    pub const fn managing(self) -> Self {
        let manager = self.until(Relation::Manager);
        let [_, _, _, _, owner] = self.0;
        Self([manager, manager, manager, manager, owner])
    }

    /// The rows that describe these levels after `now_ms`: the strongest
    /// relation first, then each weaker one that outlives every stronger
    /// one. `None` means it never expires.
    #[must_use]
    pub fn rows(&self, now_ms: i64) -> Vec<(Relation, Option<i64>)> {
        let mut out = Vec::new();
        let mut best = now_ms;
        for r in Relation::ALL.into_iter().rev() {
            let t = self.until(r);
            if t > best {
                out.push((r, (t != FOREVER).then_some(t)));
                best = t;
            }
        }
        out
    }
}

/// Effective access on one object, per principal.
pub type Access = BTreeMap<Principal, Levels>;

/// Everything materialisation needs to know about one space.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpaceGraph {
    /// The space.
    pub space: Id,
    /// The space's authz epoch when this graph was read.
    pub epoch: u64,
    /// Each node's parent node, from the depth-1 rows of `page_ancestors`;
    /// `None` for a node at the top of the space.
    pub parents: BTreeMap<Id, Option<Id>>,
    /// The space's tuples: role grants on the space and its nodes, and
    /// `restricted` markers.
    pub tuples: Vec<Tuple>,
}

impl SpaceGraph {
    /// The node and every node below it.
    #[must_use]
    pub fn subtree(&self, root: Id) -> BTreeSet<Id> {
        let children = self.children();
        let mut out = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if self.parents.contains_key(&node) && out.insert(node) {
                stack.extend(
                    children.get(&Some(node)).into_iter().flatten().copied(),
                );
            }
        }
        out
    }

    fn children(&self) -> BTreeMap<Option<Id>, Vec<Id>> {
        let mut children: BTreeMap<Option<Id>, Vec<Id>> = BTreeMap::new();
        for (&node, &parent) in &self.parents {
            children.entry(parent).or_default().push(node);
        }
        children
    }
}

/// Group membership across the workspace: `group#member@subject` tuples,
/// each with the time it stops counting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directory {
    members: BTreeMap<Id, BTreeMap<Subject, i64>>,
}

impl Directory {
    /// Builds the directory from membership tuples, dropping any that are
    /// not memberships or have already expired at `now_ms`.
    #[must_use]
    pub fn new(tuples: &[Tuple], now_ms: i64) -> Self {
        let mut members: BTreeMap<Id, BTreeMap<Subject, i64>> = BTreeMap::new();
        for t in tuples {
            if let (Object::Group(group), TupleRelation::Member) =
                (t.object, t.relation)
                && t.is_live(now_ms)
            {
                let slot = members
                    .entry(group)
                    .or_default()
                    .entry(t.subject)
                    .or_insert(NEVER);
                *slot = (*slot).max(until(t.expires_at));
            }
        }
        Self { members }
    }

    /// Every principal in a group, through nested groups, with the time
    /// until which they are a member: the best path, where a path lasts
    /// until its earliest expiring membership. Cycles are harmless.
    #[must_use]
    pub fn expand(&self, group: Id) -> BTreeMap<Principal, i64> {
        let mut best: BTreeMap<Id, i64> = BTreeMap::from([(group, FOREVER)]);
        let mut out: BTreeMap<Principal, i64> = BTreeMap::new();
        let mut stack = vec![group];
        while let Some(g) = stack.pop() {
            let reach = best.get(&g).copied().unwrap_or(NEVER);
            for (subject, &edge) in self.members.get(&g).into_iter().flatten() {
                let t = reach.min(edge);
                match (subject.principal(), subject) {
                    (Some(p), _) => {
                        let slot = out.entry(p).or_insert(NEVER);
                        *slot = (*slot).max(t);
                    }
                    (None, Subject::Group(inner)) => {
                        if best.get(inner).is_none_or(|&old| t > old) {
                            let _ = best.insert(*inner, t);
                            stack.push(*inner);
                        }
                    }
                    (None, _) => {}
                }
            }
        }
        out
    }

    /// Every group `principal` belongs to, directly or through nested
    /// groups, counting only memberships still live when the directory
    /// was built.
    #[must_use]
    pub fn groups_of(&self, principal: Principal) -> BTreeSet<Id> {
        let subject = Subject::from(principal);
        self.members
            .iter()
            .filter(|(_, subjects)| subjects.contains_key(&subject))
            .flat_map(|(&group, _)| self.containing(group))
            .collect()
    }

    /// Every group that contains `group`, directly or through nesting,
    /// including `group` itself.
    #[must_use]
    pub fn containing(&self, group: Id) -> BTreeSet<Id> {
        let mut out = BTreeSet::from([group]);
        loop {
            let before = out.len();
            for (&g, subjects) in &self.members {
                if subjects.keys().any(|s| {
                    matches!(
                        s,
                        Subject::Group(inner) if out.contains(inner)
                    )
                }) {
                    let _ = out.insert(g);
                }
            }
            if out.len() == before {
                return out;
            }
        }
    }
}

/// Why a space could not be materialised.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// A node names a parent that is not in the space.
    #[error("node {node} has parent {parent}, which is not in the space")]
    DanglingParent {
        /// The child.
        node: Id,
        /// The missing parent.
        parent: Id,
    },
    /// The parent links loop, so some nodes have no path to the space.
    #[error("the folder tree has a cycle through node {0}")]
    Cycle(Id),
}

/// The effective access of a whole space.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Materialised {
    /// Access on the space object itself.
    pub space: Access,
    /// Access on each node.
    pub nodes: BTreeMap<Id, Access>,
}

impl Materialised {
    /// Access on one object of the space; `None` if it is not in the space.
    #[must_use]
    pub fn on(&self, space: Id, object: Id) -> Option<&Access> {
        if object == space {
            Some(&self.space)
        } else {
            self.nodes.get(&object)
        }
    }
}

/// Grants on one object, with groups expanded to principals.
fn expand_grants<'a>(
    tuples: impl Iterator<Item = &'a Tuple>,
    directory: &Directory,
    now_ms: i64,
) -> Access {
    let mut access = Access::new();
    for t in tuples.filter(|t| t.is_live(now_ms)) {
        let TupleRelation::Role(relation) = t.relation else {
            continue;
        };
        let grant_until = until(t.expires_at);
        let principals: BTreeMap<Principal, i64> = match t.subject {
            Subject::Group(g) => directory.expand(g),
            other => other
                .principal()
                .into_iter()
                .map(|p| (p, FOREVER))
                .collect(),
        };
        for (p, member_until) in principals {
            raise(
                &mut access,
                p,
                Levels::granting(relation, grant_until.min(member_until)),
            );
        }
    }
    access
}

fn raise(access: &mut Access, principal: Principal, levels: Levels) {
    let slot = access.entry(principal).or_default();
    *slot = slot.or(levels);
}

/// Computes effective access for every node of a space, exact for every
/// time from `now_ms` on.
///
/// # Errors
///
/// Returns [`GraphError`] when the parent links do not form a forest.
pub fn materialise(
    graph: &SpaceGraph,
    directory: &Directory,
    now_ms: i64,
) -> Result<Materialised, GraphError> {
    for (&node, parent) in &graph.parents {
        if let Some(parent) = *parent
            && !graph.parents.contains_key(&parent)
        {
            return Err(GraphError::DanglingParent { node, parent });
        }
    }
    let mut on_object: BTreeMap<Object, Vec<&Tuple>> = BTreeMap::new();
    let mut restricted = BTreeSet::new();
    for t in &graph.tuples {
        match (t.object, t.relation) {
            // A restriction that expires would widen access later; treat
            // it as permanent until it is removed, which fails closed.
            (Object::Node(n), TupleRelation::Restricted) => {
                let _ = restricted.insert(n);
            }
            (object, TupleRelation::Role(_)) => {
                on_object.entry(object).or_default().push(t);
            }
            _ => {}
        }
    }
    let grants = |object: Object| {
        expand_grants(
            on_object.get(&object).into_iter().flatten().copied(),
            directory,
            now_ms,
        )
    };

    let space = prune(grants(Object::Space(graph.space)), now_ms);
    let children = graph.children();
    let mut nodes = BTreeMap::new();
    // (node, the parent's access, the ceiling in force above the node)
    let mut frames: Vec<(Id, Access, Option<Access>)> = Vec::new();
    for &top in children.get(&None).into_iter().flatten() {
        frames.push((top, space.clone(), None));
    }
    while let Some((node, parent_access, ceiling)) = frames.pop() {
        let own = grants(Object::Node(node));
        let (access, ceiling) = if restricted.contains(&node) {
            let mut access: Access = parent_access
                .iter()
                .map(|(p, l)| (*p, l.managing()))
                .collect();
            for (p, l) in own {
                if let Some(above) = parent_access.get(&p) {
                    raise(&mut access, p, l.and(*above));
                }
            }
            (prune(access, now_ms), Some(parent_access))
        } else {
            let mut access = parent_access;
            for (p, l) in own {
                match &ceiling {
                    None => raise(&mut access, p, l),
                    Some(cap) => {
                        if let Some(limit) = cap.get(&p) {
                            raise(&mut access, p, l.and(*limit));
                        }
                    }
                }
            }
            (prune(access, now_ms), ceiling)
        };
        for &child in children.get(&Some(node)).into_iter().flatten() {
            frames.push((child, access.clone(), ceiling.clone()));
        }
        let _ = nodes.insert(node, access);
    }
    if let Some((&node, _)) =
        graph.parents.iter().find(|(n, _)| !nodes.contains_key(n))
    {
        return Err(GraphError::Cycle(node));
    }
    Ok(Materialised { space, nodes })
}

fn prune(mut access: Access, now_ms: i64) -> Access {
    access.retain(|_, l| !l.is_dead(now_ms));
    access
}

/// The earliest expiry after `now_ms` among `tuples`: when a sweep can
/// next delete something. Sweeps only collect garbage; access is already
/// exact without them.
#[must_use]
pub fn next_expiry<'a>(
    tuples: impl IntoIterator<Item = &'a Tuple>,
    now_ms: i64,
) -> Option<i64> {
    tuples
        .into_iter()
        .filter_map(|t| t.expires_at)
        .filter(|&at| at > now_ms)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: Id = Id(1000);
    const ALICE: Principal = Principal::User(Id(1));
    const BOB: Principal = Principal::User(Id(2));

    fn graph(
        parents: &[(u128, Option<u128>)],
        tuples: Vec<Tuple>,
    ) -> SpaceGraph {
        SpaceGraph {
            space: SPACE,
            epoch: 0,
            parents: parents.iter().map(|(n, p)| (Id(*n), p.map(Id))).collect(),
            tuples,
        }
    }

    fn node(n: u128, r: Relation, s: Principal) -> Tuple {
        Tuple::role(Object::Node(Id(n)), r, s.into())
    }

    #[test]
    fn dangling_parent_and_cycle_are_errors() {
        let g = graph(&[(1, Some(9))], vec![]);
        assert_eq!(
            materialise(&g, &Directory::default(), 0),
            Err(GraphError::DanglingParent {
                node: Id(1),
                parent: Id(9)
            })
        );
        let g = graph(&[(1, Some(2)), (2, Some(1))], vec![]);
        assert!(matches!(
            materialise(&g, &Directory::default(), 0),
            Err(GraphError::Cycle(_))
        ));
    }

    #[test]
    fn restriction_narrows_but_managers_stay() {
        // space: alice manager, bob editor; 1 ⊃ 2 (restricted) ⊃ 3
        let g = graph(
            &[(1, None), (2, Some(1)), (3, Some(2))],
            vec![
                Tuple::role(
                    Object::Space(SPACE),
                    Relation::Manager,
                    ALICE.into(),
                ),
                Tuple::role(Object::Space(SPACE), Relation::Editor, BOB.into()),
                Tuple::restricted(Id(2)),
            ],
        );
        let m = materialise(&g, &Directory::default(), 0).unwrap();
        assert_eq!(
            m.nodes[&Id(1)].get(&BOB).and_then(|l| l.at(0)),
            Some(Relation::Editor)
        );
        assert_eq!(m.nodes[&Id(2)].get(&BOB).and_then(|l| l.at(0)), None);
        assert_eq!(m.nodes[&Id(3)].get(&BOB).and_then(|l| l.at(0)), None);
        assert_eq!(
            m.nodes[&Id(3)].get(&ALICE).and_then(|l| l.at(0)),
            Some(Relation::Manager)
        );

        // Granting bob owner on the restricted node is capped at editor.
        let mut g = g;
        g.tuples.push(node(2, Relation::Owner, BOB));
        let m = materialise(&g, &Directory::default(), 0).unwrap();
        assert_eq!(
            m.nodes[&Id(3)].get(&BOB).and_then(|l| l.at(0)),
            Some(Relation::Editor)
        );
    }

    #[test]
    fn subtree_and_containing_groups() {
        let g =
            graph(&[(1, None), (2, Some(1)), (3, Some(2)), (4, None)], vec![]);
        assert_eq!(g.subtree(Id(2)), BTreeSet::from([Id(2), Id(3)]));
        assert!(g.subtree(Id(99)).is_empty());
        let d = Directory::new(
            &[
                Tuple::member(Id(10), Subject::Group(Id(11))),
                Tuple::member(Id(11), Subject::Group(Id(10))),
                Tuple::member(Id(11), ALICE.into()),
            ],
            0,
        );
        assert_eq!(d.expand(Id(10)), BTreeMap::from([(ALICE, FOREVER)]));
        assert_eq!(d.containing(Id(11)), BTreeSet::from([Id(10), Id(11)]));
    }

    #[test]
    fn levels_rows_and_paths() {
        let l = Levels::granting(Relation::Editor, 50)
            .or(Levels::granting(Relation::Viewer, FOREVER));
        assert_eq!(l.at(10), Some(Relation::Editor));
        assert_eq!(l.at(50), Some(Relation::Viewer));
        assert_eq!(
            l.rows(10),
            vec![(Relation::Editor, Some(50)), (Relation::Viewer, None)]
        );
        assert_eq!(l.rows(60), vec![(Relation::Viewer, None)]);
        assert!(Levels::granting(Relation::Owner, 5).is_dead(5));
        // alice is in 10 via 11 until 30 (11 ⊂ 10 until 30, alice ⊂ 11 until
        // 90), and directly until 20: the best path wins.
        let d = Directory::new(
            &[
                Tuple::member(Id(10), Subject::Group(Id(11))).expiring(30),
                Tuple::member(Id(11), ALICE.into()).expiring(90),
                Tuple::member(Id(10), ALICE.into()).expiring(20),
            ],
            0,
        );
        assert_eq!(d.expand(Id(10)), BTreeMap::from([(ALICE, 30)]));
        assert_eq!(d.expand(Id(11)), BTreeMap::from([(ALICE, 90)]));
    }

    #[test]
    fn next_expiry_ignores_past_and_permanent() {
        let t =
            Tuple::role(Object::Space(SPACE), Relation::Viewer, ALICE.into());
        let ts = [t, t.expiring(5), t.expiring(50), t.expiring(20)];
        assert_eq!(next_expiry(&ts, 10), Some(20));
        assert_eq!(next_expiry(&ts, 60), None);
    }
}
