// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! "Convert headings to sections" and the inverse (R-BK-6).
//!
//! Splitting turns each heading of one level in a section into a child
//! section holding the text under it; deeper headings move along and are
//! promoted so the first level below the new title is `##`. Merging folds
//! a section's children back into it as headings, depth mapping to level.
//! Links follow both ways: `vellum://doc/<parent>#<anchor>` becomes a link
//! to the new section, and links to merged sections become links to their
//! heading. Links from outside the book are reported as [`Redirect`]s for
//! the index to resolve.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::{IdSource, NodeId};
use crate::markdown::{self, HeadingRef, LinkEdit, LinkKind};
use crate::ops::{NodeRef, Place};

/// Where a link to a section that no longer exists should now go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    /// The section that now holds the content.
    pub id: NodeId,
    /// The heading anchor within it, if any.
    pub anchor: Option<String>,
}

impl Redirect {
    /// The `vellum://doc/<id>#<anchor>` link for this redirect.
    #[must_use]
    pub fn uri(&self) -> String {
        self.anchor.as_ref().map_or_else(
            || self.id.uri(),
            |anchor| format!("{}#{anchor}", self.id.uri()),
        )
    }
}

/// Shifts every heading in `body` by `delta` levels (clamped to 1..=6),
/// rewriting setext headings as ATX headings.
fn shift_headings(body: &str, delta: isize) -> String {
    if delta == 0 {
        return body.to_owned();
    }
    let edits = markdown::headings(body)
        .into_iter()
        .map(|h| {
            let level = (isize::from(h.level) + delta).clamp(1, 6);
            let level = usize::try_from(level).unwrap_or(1);
            (h.range, markdown::heading_line(level, &h.source))
        })
        .collect();
    markdown::apply(body, edits)
}

impl Book {
    /// Splits section `id` at its headings of `level` (the shallowest
    /// level present when `None`), creating one child section per heading,
    /// placed before any existing children. Returns the new ids.
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist, has no headings
    /// at that level, or the children would nest deeper than `max_depth`.
    pub fn split_headings(
        &mut self,
        id: &NodeId,
        level: Option<u8>,
        ids: &mut dyn IdSource,
    ) -> Result<Vec<NodeId>> {
        let text = self.text(id)?.to_owned();
        let headings = markdown::headings(&text);
        let level = level
            .or_else(|| headings.iter().map(|h| h.level).min())
            .ok_or_else(|| BookError::NotAllowed(format!("section {id} has no headings")))?;
        let cuts: Vec<&HeadingRef> = headings.iter().filter(|h| h.level == level).collect();
        let Some(first) = cuts.first() else {
            return Err(BookError::NotAllowed(format!(
                "section {id} has no level-{level} headings"
            )));
        };
        let slot = self.slot_of(id)?;
        let max = self.manifest.max_depth;
        if slot.depth() + 1 > usize::from(max) {
            return Err(BookError::TooDeep {
                max,
                needed: slot.depth() + 1,
            });
        }
        let keep = text
            .get(..first.range.start)
            .unwrap_or_default()
            .trim_end()
            .to_owned();
        let delta = 1 - isize::from(level);
        // Plan each new section before changing anything.
        let mut pieces = Vec::new();
        for (index, cut) in cuts.iter().enumerate() {
            let end = cuts
                .get(index + 1)
                .map_or(text.len(), |next| next.range.start);
            let body = text.get(cut.range.end..end).unwrap_or_default();
            let anchors: BTreeSet<String> = markdown::headings(body)
                .into_iter()
                .map(|h| h.anchor)
                .collect();
            pieces.push((
                cut.text.clone(),
                cut.anchor.clone(),
                shift_headings(body, delta),
                anchors,
            ));
        }
        let mut new_ids = Vec::new();
        let mut redirects: BTreeMap<String, Redirect> = BTreeMap::new();
        let mut previous: Option<NodeId> = None;
        for (title, anchor, body, anchors) in pieces {
            let place = previous.map_or_else(
                || Place::FirstChildOf(NodeRef::Id(id.clone())),
                |prev| Place::After(NodeRef::Id(prev)),
            );
            let new = self.add_section(&place, &title, &body, ids)?;
            let _ = redirects.insert(
                anchor,
                Redirect {
                    id: new.clone(),
                    anchor: None,
                },
            );
            for sub in anchors {
                let _ = redirects.entry(sub.clone()).or_insert_with(|| Redirect {
                    id: new.clone(),
                    anchor: Some(sub),
                });
            }
            previous = Some(new.clone());
            new_ids.push(new);
        }
        let mut parent_text = keep;
        parent_text.push('\n');
        let path = self.path_of(id)?.clone();
        let _ = self.files.insert(path, parent_text);
        self.retarget_split(id, &new_ids, &redirects);
        Ok(new_ids)
    }

    /// Folds every section under `id` back into it as headings, in reading
    /// order; a child becomes `##`, a grandchild `###`, and their own
    /// headings move down to match. Returns where links to each removed
    /// section should now go.
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist, has no children,
    /// or a heading would need a level deeper than 6.
    pub fn merge_into_headings(&mut self, id: &NodeId) -> Result<BTreeMap<NodeId, Redirect>> {
        let slot = self.slot_of(id)?;
        let chapter = self
            .summary
            .chapter(&slot)
            .cloned()
            .ok_or_else(|| BookError::UnknownNode(id.clone()))?;
        if chapter.children.is_empty() {
            return Err(BookError::NotAllowed(format!(
                "section {id} has no children"
            )));
        }
        let mut used: BTreeSet<String> = markdown::headings(self.text(id)?)
            .into_iter()
            .map(|h| h.anchor)
            .collect();
        let mut appended = String::new();
        let mut redirects = BTreeMap::new();
        let mut stack: Vec<(usize, &crate::summary::Chapter)> =
            chapter.children.iter().rev().map(|c| (1, c)).collect();
        while let Some((depth, child)) = stack.pop() {
            let level = depth + 1;
            if level > 6 {
                return Err(BookError::NotAllowed(format!(
                    "{} would need heading level {level}; the limit is 6",
                    child.name
                )));
            }
            let base = markdown::anchor(&child.name);
            let mut anchor = base.clone();
            let mut n = 1;
            while used.contains(&anchor) {
                n += 1;
                anchor = format!("{base}-{n}");
            }
            let _ = used.insert(anchor.clone());
            let heading = if anchor == base {
                child.name.clone()
            } else {
                format!("{} {{#{anchor}}}", child.name)
            };
            let _ = write!(appended, "\n{}", markdown::heading_line(level, &heading));
            if let Some(child_id) = child.location.as_ref().and_then(|p| self.id_at(p)) {
                let body = markdown::body(self.text(child_id)?);
                let max_inner = markdown::headings(body)
                    .iter()
                    .map(|h| h.level)
                    .max()
                    .unwrap_or(0);
                if usize::from(max_inner) + depth > 6 {
                    return Err(BookError::NotAllowed(format!(
                        "headings in {} would go deeper than level 6",
                        child.name
                    )));
                }
                let shifted = shift_headings(body, isize::try_from(depth).unwrap_or(0));
                let shifted = shifted.trim_matches('\n');
                if !shifted.is_empty() {
                    appended.push('\n');
                    appended.push_str(shifted);
                    appended.push('\n');
                }
                let _ = redirects.insert(
                    child_id.clone(),
                    Redirect {
                        id: id.clone(),
                        anchor: Some(anchor),
                    },
                );
            }
            stack.extend(child.children.iter().rev().map(|c| (depth + 1, c)));
        }
        let mut text = self.text(id)?.trim_end().to_owned();
        text.push('\n');
        text.push_str(&appended);
        // Remove the children (their files go to the trash list).
        while self
            .summary
            .chapter(&slot)
            .is_some_and(|c| !c.children.is_empty())
        {
            let first = slot.child(0);
            let _ = self.remove_section(&NodeRef::Slot(first))?;
        }
        let path = self.path_of(id)?.clone();
        let _ = self.files.insert(path, text);
        self.retarget_merged(&redirects);
        Ok(redirects)
    }

    /// After a split: links to `parent#anchor` (and `#anchor` inside the
    /// new sections) go to the sections that now hold the anchors.
    fn retarget_split(
        &mut self,
        id: &NodeId,
        new_ids: &[NodeId],
        redirects: &BTreeMap<String, Redirect>,
    ) {
        let new_set: BTreeSet<NodeId> = new_ids.iter().cloned().collect();
        let paths: Vec<_> = self.files.keys().cloned().collect();
        for path in paths {
            let in_piece = self.id_at(&path).is_some_and(|i| new_set.contains(i));
            let Some(text) = self.files.get(&path) else {
                continue;
            };
            let updated = markdown::map_links(text, |link, _| {
                if link.kind == LinkKind::Image {
                    return None;
                }
                let redirect =
                    if let Some(fragment) = link.dest.strip_prefix('#') {
                        if in_piece {
                            // Same-page links inside a new section: to the
                            // section now holding the anchor, or back to the parent.
                            return Some(LinkEdit {
                                dest: Some(redirects.get(fragment).map_or_else(
                                    || format!("{}#{fragment}", id.uri()),
                                    Redirect::uri,
                                )),
                                ..LinkEdit::default()
                            });
                        }
                        // Same-page links in the parent: only if the anchor moved.
                        redirects.get(fragment)?
                    } else {
                        let (target, fragment) = NodeId::from_uri(&link.dest)?;
                        if &target != id {
                            return None;
                        }
                        redirects.get(fragment?)?
                    };
                Some(LinkEdit {
                    dest: Some(redirect.uri()),
                    ..LinkEdit::default()
                })
            });
            let _ = self.files.insert(path, updated);
        }
    }

    /// After a merge: links to removed sections go to their headings.
    fn retarget_merged(&mut self, redirects: &BTreeMap<NodeId, Redirect>) {
        let paths: Vec<_> = self.files.keys().cloned().collect();
        for path in paths {
            let Some(text) = self.files.get(&path) else {
                continue;
            };
            let updated = markdown::map_links(text, |link, _| {
                let (target, fragment) = NodeId::from_uri(&link.dest)?;
                let redirect: &Redirect = redirects.get(&target)?;
                let dest = fragment
                    .map_or_else(|| redirect.uri(), |f| format!("{}#{f}", redirect.id.uri()));
                Some(LinkEdit {
                    dest: Some(dest),
                    ..LinkEdit::default()
                })
            });
            let _ = self.files.insert(path, updated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};
    use crate::id::SequentialIds;

    #[test]
    fn split_and_merge_round_trip() {
        let mut book = handbook();
        let mut ids = SequentialIds::new("01JBK00004");
        let new = book.split_headings(&id(3), None, &mut ids).unwrap();
        assert_eq!(new.len(), 2);
        let titles: Vec<_> = new.iter().map(|n| book.label_of(n).unwrap()).collect();
        assert_eq!(titles, ["§1.1.1 Day one", "§1.1.2 Day two"]);
        assert_eq!(
            book.path_of(&id(3)).unwrap().as_str(),
            "onboarding/first-week/README.md"
        );
        assert_eq!(
            book.path_of(new.first().unwrap()).unwrap().as_str(),
            "onboarding/first-week/day-one.md"
        );
        assert_eq!(
            markdown::body(book.text(new.first().unwrap()).unwrap()).trim(),
            "Badge."
        );
        let week = book.text(&id(3)).unwrap();
        assert!(!week.contains("Day one"), "{week}");
        assert!(week.contains("Read [§]"), "{week}");

        let redirects = book.merge_into_headings(&id(3)).unwrap();
        assert_eq!(redirects.len(), 2);
        let week = book.text(&id(3)).unwrap();
        assert!(week.contains("## Day one\n\nBadge."), "{week}");
        assert!(week.contains("## Day two\n\nLaptop."), "{week}");
        assert_eq!(
            book.path_of(&id(3)).unwrap().as_str(),
            "onboarding/first-week.md"
        );
        assert!(book.removed().len() >= 2);
    }

    #[test]
    fn links_follow_the_split() {
        let mut book = handbook();
        // Glossary links to a heading of the week page.
        let glossary = format!(
            "{}\nSee [day two]({}#day-two).\n",
            book.text(&id(6)).unwrap(),
            id(3).uri()
        );
        book.set_text(&id(6), glossary).unwrap();
        let mut ids = SequentialIds::new("01JBK00004");
        let new = book.split_headings(&id(3), Some(2), &mut ids).unwrap();
        let day_two = new.get(1).unwrap();
        assert!(
            book.text(&id(6))
                .unwrap()
                .contains(&format!("[day two]({})", day_two.uri()))
        );
        // And merging sends it back to the heading.
        let _ = book.merge_into_headings(&id(3)).unwrap();
        assert!(
            book.text(&id(6))
                .unwrap()
                .contains(&format!("[day two]({}#day-two)", id(3).uri()))
        );
    }

    #[test]
    fn merge_nests_levels_and_disambiguates() {
        let mut book = handbook();
        book.set_max_depth(4).unwrap();
        let mut ids = SequentialIds::new("01JBK00004");
        let a = book
            .add_section(
                &Place::LastChildOf(id(5).into()),
                "Gateway",
                "## Limits\n\nFast.",
                &mut ids,
            )
            .unwrap();
        let _ = book
            .add_section(
                &Place::LastChildOf(a.into()),
                "Gateway",
                "Nested.",
                &mut ids,
            )
            .unwrap();
        let _ = book.merge_into_headings(&id(4)).unwrap();
        let text = book.text(&id(4)).unwrap();
        assert!(text.contains("## Services"), "{text}");
        assert!(text.contains("### Gateway\n"), "{text}");
        assert!(text.contains("#### Limits"), "{text}");
        assert!(text.contains("#### Gateway {#gateway-2}"), "{text}");
        assert!(book.merge_into_headings(&id(6)).is_err());
    }

    #[test]
    fn shifting() {
        assert_eq!(shift_headings("Title\n===\n\nx\n", 1), "## Title\n\nx\n");
        assert_eq!(shift_headings("### Deep\n", -1), "## Deep\n");
    }
}
