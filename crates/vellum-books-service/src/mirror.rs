// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Mirroring section restrictions into the authorization engine (D144).
//!
//! The index registers every chapter of a book as a node under the book
//! (its front-matter ID, in `page_ancestors`). For each section whose
//! effective `restrict_to` is set, the engine gets a `restricted` marker
//! and one [`GRANTED`] grant per principal in that set. The engine's
//! narrowing rule then gives exactly the book's own rule: someone who can
//! open the book sees the section only if a grant names them (directly or
//! through a group), capped at their relation on the book, while book
//! managers and owners keep access.
//!
//! The effective set is the literal intersection of every enclosing
//! restriction (books refuse a child that widens its parent), so a flat
//! tree of sections under the book is exact.
//!
//! [`plan`] is pure: it compares what `book.yaml` asks for with the
//! tuples already on the book's section nodes and returns the changes.
//! Every role grant on a section node belongs to the mirror; sections are
//! not shared on their own.

use std::collections::{BTreeMap, BTreeSet};

use vellum_authz::{Change, Id, Object, Relation, SpaceGraph, Subject, Tuple, TupleRelation};
use vellum_books::manifest::Principal;
use vellum_books::{Book, BookError, NodeId};
use vellum_kernel::ObjectId;

/// The role each named principal gets on a restricted section. The
/// engine caps it at what they hold on the book, so a viewer of the book
/// stays a viewer and an editor stays an editor.
pub const GRANTED: Relation = Relation::Editor;

/// What one book asks of the authorization engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restrictions {
    /// Every section of the book.
    pub sections: BTreeSet<Id>,
    /// The restricted sections and who may see each.
    pub restricted: BTreeMap<Id, BTreeSet<Subject>>,
}

impl Restrictions {
    /// Reads them from a book.
    ///
    /// A principal that is not `user:<ULID>` or `group:<ULID>` cannot be
    /// granted anything, so it is left out: the section stays restricted
    /// and fewer people see it.
    ///
    /// # Errors
    ///
    /// Only for an inconsistent book.
    pub fn of(book: &Book) -> Result<Self, BookError> {
        let sections = book.manifest().nodes.keys().filter_map(node_id).collect();
        let restricted = book
            .restrictions()?
            .iter()
            .filter_map(|(section, allowed)| {
                let subjects = allowed.iter().filter_map(subject_of).collect();
                node_id(section).map(|id| (id, subjects))
            })
            .collect();
        Ok(Self {
            sections,
            restricted,
        })
    }
}

/// The engine ID of a section.
#[must_use]
pub fn node_id(id: &NodeId) -> Option<Id> {
    id.as_str()
        .parse::<ObjectId>()
        .ok()
        .map(|o| Id(o.as_u128()))
}

/// The engine subject for a book principal, if it names a user or group
/// by ULID.
#[must_use]
pub fn subject_of(principal: &Principal) -> Option<Subject> {
    let (kind, name) = principal.parts();
    let id = Id(name.parse::<ObjectId>().ok()?.as_u128());
    match kind {
        "user" => Some(Subject::User(id)),
        "group" => Some(Subject::Group(id)),
        _ => None,
    }
}

/// The changes that bring the engine in line with `wanted`, and how many
/// sections were skipped because the engine's tree does not hold them
/// yet (the index has not seen the file).
#[must_use]
pub fn plan(graph: &SpaceGraph, wanted: &Restrictions) -> (Vec<Change>, usize) {
    let mut changes = Vec::new();
    let mut skipped = 0;
    for &section in &wanted.sections {
        if !graph.parents.contains_key(&section) {
            skipped += 1;
            continue;
        }
        let node = Object::Node(section);
        let mut restricted_now = false;
        let mut grants_now = BTreeSet::new();
        for t in graph.tuples.iter().filter(|t| t.object == node) {
            match t.relation {
                TupleRelation::Restricted => restricted_now = true,
                TupleRelation::Role(role) => {
                    let _ = grants_now.insert((role, t.subject));
                }
                TupleRelation::Member => {}
            }
        }
        let allowed = wanted.restricted.get(&section);
        let grants_wanted: BTreeSet<(Relation, Subject)> = allowed
            .into_iter()
            .flatten()
            .map(|subject| (GRANTED, *subject))
            .collect();
        match (allowed.is_some(), restricted_now) {
            (true, false) => changes.push(Change::Restrict(section)),
            (false, true) => changes.push(Change::Unrestrict(section)),
            _ => {}
        }
        for (relation, subject) in grants_now.difference(&grants_wanted) {
            changes.push(Change::Revoke {
                object: node,
                relation: *relation,
                subject: *subject,
            });
        }
        for (relation, subject) in grants_wanted.difference(&grants_now) {
            changes.push(Change::Grant(Tuple::role(node, *relation, *subject)));
        }
    }
    (changes, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK: Id = Id(10);
    const OPEN: Id = Id(11);
    const SECRET: Id = Id(12);
    const LATER: Id = Id(13);

    fn graph(tuples: Vec<Tuple>) -> SpaceGraph {
        SpaceGraph {
            space: Id(1),
            epoch: 0,
            parents: BTreeMap::from([(BOOK, None), (OPEN, Some(BOOK)), (SECRET, Some(BOOK))]),
            tuples,
        }
    }

    fn wanted(restricted: &[(Id, &[Subject])]) -> Restrictions {
        Restrictions {
            sections: BTreeSet::from([OPEN, SECRET, LATER]),
            restricted: restricted
                .iter()
                .map(|(id, subjects)| (*id, subjects.iter().copied().collect()))
                .collect(),
        }
    }

    #[test]
    fn restricts_grants_and_skips_unindexed_sections() {
        let sre = Subject::Group(Id(50));
        let (changes, skipped) = plan(&graph(vec![]), &wanted(&[(SECRET, &[sre])]));
        assert_eq!(skipped, 1);
        assert_eq!(
            changes,
            vec![
                Change::Restrict(SECRET),
                Change::Grant(Tuple::role(Object::Node(SECRET), GRANTED, sre)),
            ]
        );
    }

    #[test]
    fn converges_and_lifts() {
        let sre = Subject::Group(Id(50));
        let ana = Subject::User(Id(60));
        let current = graph(vec![
            Tuple::restricted(SECRET),
            Tuple::role(Object::Node(SECRET), GRANTED, sre),
            Tuple::role(Object::Node(SECRET), Relation::Owner, ana),
        ]);
        // Already in line apart from a stray grant, which goes.
        let (changes, _) = plan(&current, &wanted(&[(SECRET, &[sre])]));
        assert_eq!(
            changes,
            vec![Change::Revoke {
                object: Object::Node(SECRET),
                relation: Relation::Owner,
                subject: ana,
            }]
        );
        // Lifting the restriction removes the marker and the grants.
        let (changes, _) = plan(&current, &wanted(&[]));
        assert_eq!(changes.first(), Some(&Change::Unrestrict(SECRET)));
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn principals_map_to_subjects() {
        let user = Principal::parse("user:01JBK0000000000000000000A1").unwrap();
        let group = Principal::parse("group:01JBK0000000000000000000A2").unwrap();
        assert!(matches!(subject_of(&user), Some(Subject::User(_))));
        assert!(matches!(subject_of(&group), Some(Subject::Group(_))));
        assert_eq!(subject_of(&Principal::parse("group:sre").unwrap()), None);
    }
}
