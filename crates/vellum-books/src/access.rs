// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-section restrictions, owners and reviewers (R-BK-9).
//!
//! Restrictions only ever narrow. Whoever can open the book (decided by
//! the authorization service) can see a section only if they are in the
//! `restrict_to` set of that section and of every enclosing section that
//! has one. Owners and reviewers are inherited from the nearest enclosing
//! section that names any.
//!
//! The authorization service stays the source of truth for access; it
//! reads [`SectionAccess`] to materialise the narrowing in its tuples, and
//! publishing uses it to leave restricted sections out of public sites.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::NodeId;
use crate::manifest::Principal;
use crate::publish::Audience;

/// The effective access settings of one section.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct SectionAccess {
    /// Who may see the section, when any enclosing section restricts it:
    /// the intersection of every restriction on the way down.
    pub restrict_to: Option<BTreeSet<Principal>>,
    /// The sections whose restrictions apply, outermost first.
    pub restricted_by: Vec<NodeId>,
    /// The effective owners.
    pub owners: BTreeSet<Principal>,
    /// The effective reviewers.
    pub reviewers: BTreeSet<Principal>,
}

impl SectionAccess {
    /// Whether someone holding `principals` (their user and groups), who
    /// can open the book, may see the section.
    #[must_use]
    pub fn permits(&self, principals: &BTreeSet<Principal>) -> bool {
        self.restrict_to
            .as_ref()
            .is_none_or(|allowed| !allowed.is_disjoint(principals))
    }

    /// Whether the section is restricted at all.
    #[must_use]
    pub const fn is_restricted(&self) -> bool {
        self.restrict_to.is_some()
    }
}

impl Book {
    /// The effective access settings of section `id`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn access(&self, id: &NodeId) -> Result<SectionAccess> {
        let slot = self.slot_of(id)?;
        let mut chain: Vec<NodeId> = Self::ancestors(&slot)
            .iter()
            .filter_map(|s| self.id_of_slot(s).cloned())
            .collect();
        chain.push(id.clone());
        let mut access = SectionAccess::default();
        for node in &chain {
            let Some(entry) = self.manifest.nodes.get(node) else {
                continue;
            };
            if let Some(allowed) = &entry.restrict_to {
                access.restrict_to = Some(access.restrict_to.take().map_or_else(
                    || allowed.clone(),
                    |current| current.intersection(allowed).cloned().collect(),
                ));
                access.restricted_by.push(node.clone());
            }
            if !entry.owners.is_empty() {
                access.owners.clone_from(&entry.owners);
            }
            if !entry.reviewers.is_empty() {
                access.reviewers.clone_from(&entry.reviewers);
            }
        }
        Ok(access)
    }

    /// Whether someone holding `principals` may see section `id`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn can_view(&self, id: &NodeId, principals: &BTreeSet<Principal>) -> Result<bool> {
        Ok(self.access(id)?.permits(principals))
    }

    /// The book as someone holding `principals` (their user and groups) may
    /// see it: every section they cannot see is left out, with its file,
    /// and nothing else moves. Use it whenever book content leaves the
    /// book for a reader, such as a copy into their drive or an export.
    ///
    /// Book managers and owners see every section; callers give them the
    /// whole book instead.
    ///
    /// # Errors
    ///
    /// Returns an error only for an inconsistent book.
    pub fn view_for(&self, principals: &BTreeSet<Principal>) -> Result<Self> {
        self.published_view(&Audience::Members(principals.clone()))
    }

    /// The effective restriction of every section that has one, by
    /// section: what the authorization service mirrors as `restricted`
    /// markers and grants.
    ///
    /// # Errors
    ///
    /// Returns an error only for an inconsistent book.
    pub fn restrictions(&self) -> Result<BTreeMap<NodeId, BTreeSet<Principal>>> {
        let mut out = BTreeMap::new();
        for id in self.manifest.nodes.keys() {
            if let Some(allowed) = self.access(id)?.restrict_to {
                let _ = out.insert(id.clone(), allowed);
            }
        }
        Ok(out)
    }

    /// Restricts section `id` (and everything under it) to `allowed`, or
    /// lifts its own restriction with `None`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::WidensAccess`] when `allowed` names someone an
    /// enclosing section already excludes, [`BookError::NotAllowed`] for
    /// an empty set (use the book's own sharing to remove everyone), and
    /// [`BookError::UnknownNode`] for an unknown section.
    pub fn set_restriction(
        &mut self,
        id: &NodeId,
        allowed: Option<BTreeSet<Principal>>,
    ) -> Result<()> {
        let slot = self.slot_of(id)?;
        if let Some(allowed) = &allowed {
            if allowed.is_empty() {
                return Err(BookError::NotAllowed(
                    "a restriction must name at least one principal".to_owned(),
                ));
            }
            let enclosing = Self::ancestors(&slot)
                .last()
                .and_then(|parent| self.id_of_slot(parent).cloned());
            if let Some(parent) = enclosing {
                let outer = self.access(&parent)?;
                if let Some(outer) = outer.restrict_to
                    && let Some(extra) = allowed.difference(&outer).next()
                {
                    return Err(BookError::WidensAccess {
                        node: id.clone(),
                        principal: extra.to_string(),
                    });
                }
            }
        }
        let entry = self
            .manifest
            .nodes
            .get_mut(id)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))?;
        entry.restrict_to = allowed;
        Ok(())
    }

    /// Sets the owners of section `id`; an empty set inherits them.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] for an unknown section.
    pub fn set_owners(&mut self, id: &NodeId, owners: BTreeSet<Principal>) -> Result<()> {
        self.manifest
            .nodes
            .get_mut(id)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))?
            .owners = owners;
        Ok(())
    }

    /// Sets the reviewers of section `id`; an empty set inherits them.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] for an unknown section.
    pub fn set_reviewers(&mut self, id: &NodeId, reviewers: BTreeSet<Principal>) -> Result<()> {
        self.manifest
            .nodes
            .get_mut(id)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))?
            .reviewers = reviewers;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};

    fn set(items: &[&str]) -> BTreeSet<Principal> {
        items.iter().map(|p| Principal::parse(p).unwrap()).collect()
    }

    #[test]
    fn restrictions_narrow_down_the_tree() {
        let mut book = handbook();
        book.set_restriction(&id(4), Some(set(&["group:staff", "group:sre"])))
            .unwrap();
        book.set_restriction(&id(5), Some(set(&["group:sre"])))
            .unwrap();
        let maya = set(&["user:maya", "group:staff"]);
        let raj = set(&["user:raj", "group:sre"]);
        assert!(book.can_view(&id(4), &maya).unwrap());
        assert!(!book.can_view(&id(5), &maya).unwrap());
        assert!(book.can_view(&id(5), &raj).unwrap());
        assert!(book.can_view(&id(6), &maya).unwrap());
        let access = book.access(&id(5)).unwrap();
        assert_eq!(access.restricted_by, vec![id(4), id(5)]);
        // Widening under a restricted parent is refused.
        assert!(matches!(
            book.set_restriction(&id(5), Some(set(&["group:everyone"]))),
            Err(BookError::WidensAccess { .. })
        ));
        assert!(book.set_restriction(&id(5), Some(BTreeSet::new())).is_err());
        // Lifting the child's restriction still leaves the parent's.
        book.set_restriction(&id(5), None).unwrap();
        assert!(book.can_view(&id(5), &maya).unwrap());
        assert!(!book.can_view(&id(5), &set(&["user:guest"])).unwrap());
        // The manifest round-trips the settings.
        let yaml = book.manifest_text().unwrap();
        assert!(yaml.contains("restrict_to"), "{yaml}");
    }

    #[test]
    fn view_for_and_restrictions() {
        let mut book = handbook();
        book.set_restriction(&id(4), Some(set(&["group:staff", "group:sre"])))
            .unwrap();
        book.set_restriction(&id(5), Some(set(&["group:sre"])))
            .unwrap();
        let restrictions = book.restrictions().unwrap();
        assert_eq!(
            restrictions.get(&id(4)),
            Some(&set(&["group:sre", "group:staff"]))
        );
        assert_eq!(restrictions.get(&id(5)), Some(&set(&["group:sre"])));
        assert!(!restrictions.contains_key(&id(6)));
        assert!(!restrictions.contains_key(&id(1)));
        // A child inherits its parent's restriction.
        book.set_restriction(&id(5), None).unwrap();
        assert_eq!(
            book.restrictions().unwrap().get(&id(5)),
            Some(&set(&["group:sre", "group:staff"]))
        );
        book.set_restriction(&id(5), Some(set(&["group:sre"])))
            .unwrap();

        let staff = book.view_for(&set(&["user:maya", "group:staff"])).unwrap();
        assert!(staff.contains(&id(4)));
        assert!(!staff.contains(&id(5)));
        assert!(
            !staff
                .manifest_text()
                .unwrap()
                .contains(book.path_of(&id(5)).unwrap().as_str())
        );
        let outsider = book.view_for(&set(&["user:guest"])).unwrap();
        assert!(!outsider.contains(&id(4)));
        assert!(outsider.contains(&id(1)));
    }

    #[test]
    fn principals_from_ids() {
        let user = Principal::user("01JBK0000000000000000000A1").unwrap();
        assert_eq!(user.as_str(), "user:01JBK0000000000000000000A1");
        assert_eq!(user.parts(), ("user", "01JBK0000000000000000000A1"));
        assert_eq!(Principal::group("G1").unwrap().parts(), ("group", "G1"));
        assert!(Principal::user("").is_err());
        assert!(Principal::user("a b").is_err());
    }

    #[test]
    fn owners_and_reviewers_inherit() {
        let mut book = handbook();
        book.set_owners(&id(4), set(&["user:ana"])).unwrap();
        book.set_reviewers(&id(4), set(&["group:arch"])).unwrap();
        book.set_owners(&id(5), set(&["user:lee"])).unwrap();
        let access = book.access(&id(5)).unwrap();
        assert_eq!(access.owners, set(&["user:lee"]));
        assert_eq!(access.reviewers, set(&["group:arch"]));
    }
}
