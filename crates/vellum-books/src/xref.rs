// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Cross-references by id (R-BK-3).
//!
//! A link to `vellum://doc/<id>` whose text is empty or `§` is a
//! cross-reference: it is shown as the target's label, such as
//! `§2.3 Services`. The label is computed when the page is shown or
//! exported, so it follows every reorder without rewriting any file.

use std::collections::BTreeSet;

use crate::book::Book;
use crate::id::NodeId;
use crate::markdown::{self, LinkEdit, LinkKind};

/// Whether link text marks a cross-reference.
#[must_use]
pub fn is_cross_ref_text(text: &str) -> bool {
    let text = text.trim();
    text.is_empty() || text == "§"
}

impl Book {
    /// Fills in the text of every cross-reference to a section of this
    /// book, leaving destinations unchanged. Links to other documents are
    /// left as written.
    #[must_use]
    pub fn render_cross_refs(&self, text: &str) -> String {
        self.render_cross_refs_for(text, |_| true)
    }

    /// Like [`Book::render_cross_refs`], for a reader who may see only
    /// the sections `visible` accepts: a cross-reference to any other
    /// section keeps its `§` text, so its title never shows. Labels keep
    /// the book's own numbering, so every reader quotes the same numbers.
    #[must_use]
    pub fn render_cross_refs_for(&self, text: &str, visible: impl Fn(&NodeId) -> bool) -> String {
        markdown::map_links(text, |link, current| {
            if link.kind != LinkKind::Inline || !current.is_some_and(is_cross_ref_text) {
                return None;
            }
            let (id, _) = NodeId::from_uri(&link.dest)?;
            if !visible(&id) {
                return None;
            }
            let label = self.label_of(&id).ok()?;
            Some(LinkEdit {
                text: Some(label),
                ..LinkEdit::default()
            })
        })
    }

    /// The sections of this book that section `id` links to by id.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BookError::UnknownNode`] when the book has no such
    /// section.
    pub fn references_from(&self, id: &NodeId) -> crate::Result<BTreeSet<NodeId>> {
        let text = self.text(id)?;
        Ok(markdown::links(text)
            .into_iter()
            .filter_map(|link| NodeId::from_uri(&link.dest).map(|(id, _)| id))
            .filter(|target| self.contains(target))
            .collect())
    }

    /// The sections of this book that link to section `id` by id
    /// (backlinks within the book).
    #[must_use]
    pub fn references_to(&self, id: &NodeId) -> BTreeSet<NodeId> {
        self.manifest
            .nodes
            .keys()
            .filter(|source| {
                self.references_from(source)
                    .is_ok_and(|targets| targets.contains(id))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::book::fixtures::{handbook, id};
    use crate::ops::Place;

    #[test]
    fn labels_follow_moves() {
        let mut book = handbook();
        let week = book.text(&id(3)).unwrap().to_owned();
        let rendered = book.render_cross_refs(&week);
        assert!(
            rendered.contains("[§2.1 Services](vellum://doc/01JBK0000000000000000000A5)"),
            "{rendered}"
        );
        book.move_section(&id(4).into(), &Place::Before(id(2).into()))
            .unwrap();
        let rendered = book.render_cross_refs(&week);
        assert!(rendered.contains("[§1.1 Services]"), "{rendered}");
        // Empty link text works too, and ordinary links are untouched.
        let services = book.render_cross_refs(book.text(&id(5)).unwrap());
        assert!(
            services.contains("[§2 Onboarding](vellum://doc/"),
            "{services}"
        );
        assert!(services.contains("[the glossary](../glossary.md)"));
    }

    #[test]
    fn hidden_targets_keep_their_marker() {
        let book = handbook();
        let week = book.text(&id(3)).unwrap();
        let rendered = book.render_cross_refs_for(week, |target| target != &id(5));
        assert!(
            rendered.contains("[§](vellum://doc/01JBK0000000000000000000A5)"),
            "{rendered}"
        );
        assert!(!rendered.contains("Services"));
    }

    #[test]
    fn backlinks() {
        let book = handbook();
        assert!(book.references_from(&id(3)).unwrap().contains(&id(5)));
        assert!(book.references_to(&id(5)).contains(&id(3)));
        assert!(book.references_to(&id(6)).is_empty());
    }
}
