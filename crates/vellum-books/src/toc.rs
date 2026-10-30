// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The book-wide table of contents (R-BK-4) and the tree the export
//! service walks (R-BK-5).
//!
//! The export service asks for the whole book, a section, or a section
//! and its children, then reads the leaves in [`TocEntry::flatten`] order.
//! Every entry carries the heading level its depth maps to, so a subtree
//! exported on its own starts at level 1 ([`TocEntry::rebased`]).

use serde::Serialize;

use crate::book::Book;
use crate::id::NodeId;
use crate::numbering::{self, SectionNumber};
use crate::path::ChapterPath;
use crate::summary::{Chapter, Item, Section, Slot};

/// One chapter in the table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TocEntry {
    /// The section id; `None` for a draft.
    pub id: Option<NodeId>,
    /// Where the chapter sits in the summary.
    pub slot: Slot,
    /// The navigation title.
    pub title: String,
    /// The computed number; `None` for prefix and suffix chapters.
    pub number: Option<SectionNumber>,
    /// The number as displayed in the book's style (`2.3`, `II.C`), if any.
    pub number_label: Option<String>,
    /// The cross-reference label, such as `§2.3 Services`.
    pub label: String,
    /// The nesting depth, 1 at the top.
    pub depth: usize,
    /// The heading level the depth maps to in exports, 1 to 6.
    pub heading_level: u8,
    /// The chapter file; `None` for a draft.
    pub path: Option<ChapterPath>,
    /// The part title this chapter falls under, if any.
    pub part: Option<String>,
    /// Whether this is a draft (`[Title]()`).
    pub draft: bool,
    /// Nested chapters.
    pub children: Vec<Self>,
}

impl TocEntry {
    /// This entry and its descendants in reading order.
    #[must_use]
    pub fn flatten(&self) -> Vec<&Self> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.flatten());
        }
        out
    }

    /// A copy whose depth and heading levels start at 1 at this entry, for
    /// exporting a section on its own.
    #[must_use]
    pub fn rebased(&self) -> Self {
        let shift = self.depth.saturating_sub(1);
        let mut copy = self.clone();
        copy.shift(shift);
        copy
    }

    fn shift(&mut self, by: usize) {
        self.depth -= by;
        self.heading_level = heading_level(self.depth);
        for child in &mut self.children {
            child.shift(by);
        }
    }

    /// The entry for section `id` in this subtree.
    #[must_use]
    pub fn find(&self, id: &NodeId) -> Option<&Self> {
        if self.id.as_ref() == Some(id) {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }
}

/// A top-level line of the table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TocItem {
    /// A chapter and its children.
    Chapter(Box<TocEntry>),
    /// A part title.
    Part {
        /// The title.
        title: String,
    },
    /// A separator.
    Separator,
}

/// The table of contents of a whole book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Toc {
    /// The book id.
    pub book_id: String,
    /// The book title.
    pub title: String,
    /// The lines, in reading order.
    pub items: Vec<TocItem>,
}

impl Toc {
    /// Every chapter in reading order, at any depth.
    #[must_use]
    pub fn entries(&self) -> Vec<&TocEntry> {
        self.items
            .iter()
            .filter_map(|item| match item {
                TocItem::Chapter(entry) => Some(entry.as_ref()),
                _ => None,
            })
            .flat_map(TocEntry::flatten)
            .collect()
    }

    /// The entry for section `id` with its children.
    #[must_use]
    pub fn find(&self, id: &NodeId) -> Option<&TocEntry> {
        self.items.iter().find_map(|item| match item {
            TocItem::Chapter(entry) => entry.find(id),
            _ => None,
        })
    }
}

fn heading_level(depth: usize) -> u8 {
    u8::try_from(depth.clamp(1, 6)).unwrap_or(6)
}

impl Book {
    /// The book-wide table of contents.
    #[must_use]
    pub fn toc(&self) -> Toc {
        let numbers = self.numbers();
        let style = self.manifest.numbering;
        let entry = |slot: Slot, chapter: &Chapter, part: Option<&String>| {
            build(self, &numbers, style, slot, chapter, part)
        };
        let mut items = Vec::new();
        for section in [Section::Prefix, Section::Numbered, Section::Suffix] {
            let mut part: Option<String> = None;
            for (index, item) in self.summary.items(section).iter().enumerate()
            {
                items.push(match item {
                    Item::Chapter(chapter) => {
                        TocItem::Chapter(Box::new(entry(
                            Slot::top(section, index),
                            chapter,
                            part.as_ref(),
                        )))
                    }
                    Item::Separator => TocItem::Separator,
                    Item::Part(title) => {
                        part = Some(title.clone());
                        TocItem::Part {
                            title: title.clone(),
                        }
                    }
                });
            }
        }
        Toc {
            book_id: self.manifest.id.to_string(),
            title: self.manifest.title.clone(),
            items,
        }
    }
}

fn build(
    book: &Book,
    numbers: &std::collections::BTreeMap<Slot, SectionNumber>,
    style: crate::manifest::NumberingStyle,
    slot: Slot,
    chapter: &Chapter,
    part: Option<&String>,
) -> TocEntry {
    let number = numbers.get(&slot).cloned();
    let children = chapter
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            build(book, numbers, style, slot.child(index), child, part)
        })
        .collect();
    TocEntry {
        id: chapter
            .location
            .as_ref()
            .and_then(|path| book.id_at(path))
            .cloned(),
        depth: slot.depth(),
        heading_level: heading_level(slot.depth()),
        number_label: number.as_ref().and_then(|n| n.format(style)),
        label: numbering::label(number.as_ref(), style, &chapter.name),
        number,
        title: chapter.name.clone(),
        path: chapter.location.clone(),
        part: part.cloned(),
        draft: chapter.location.is_none(),
        children,
        slot,
    }
}

#[cfg(test)]
mod tests {
    use crate::book::fixtures::{handbook, id};

    #[test]
    fn whole_book() {
        let toc = handbook().toc();
        let labels: Vec<(&str, usize)> = toc
            .entries()
            .iter()
            .map(|e| (e.label.as_str(), e.depth))
            .collect();
        assert_eq!(
            labels,
            [
                ("Introduction", 1),
                ("§1 Onboarding", 1),
                ("§1.1 Your first week", 2),
                ("§1.2 Security training", 2),
                ("§2 Architecture", 1),
                ("§2.1 Services", 2),
                ("Glossary", 1),
            ]
        );
        let services = toc.find(&id(5)).unwrap();
        assert_eq!(services.part.as_deref(), Some("Part II · How it works"));
        let json = serde_json::to_string(&toc).unwrap();
        assert!(json.contains("\"kind\":\"part\""));
    }

    #[test]
    fn subtree_is_rebased() {
        let toc = handbook().toc();
        let onboarding = toc.find(&id(2)).unwrap().rebased();
        assert_eq!(onboarding.heading_level, 1);
        let week = onboarding.find(&id(3)).unwrap();
        assert_eq!((week.depth, week.heading_level), (2, 2));
        assert!(onboarding.flatten().iter().any(|e| e.draft));
        let week_alone = toc.find(&id(3)).unwrap().rebased();
        assert_eq!(week_alone.heading_level, 1);
    }
}
