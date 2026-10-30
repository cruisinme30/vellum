// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A book in memory: its manifest, its summary and its chapter files.
//!
//! [`Book::load`] reconciles the three after edits made outside Vellum
//! (R-DR-5, R-BK-7): renamed files are recognised by the id in their front
//! matter, files without an id get one, missing files referenced by the
//! summary are created as drafts, and manifest entries for chapters the
//! summary no longer lists are dropped. Every repair is reported as a
//! [`Repair`] so the caller can log and audit it.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{BookError, Result};
use crate::id::{BookId, IdSource, NodeId};
use crate::manifest::{Manifest, NodeEntry};
use crate::markdown;
use crate::numbering::{self, SectionNumber};
use crate::path::ChapterPath;
use crate::summary::{Chapter, Item, Section, Slot, Summary};

/// A change [`Book::load`] made to bring the files back in line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Repair {
    /// A file moved outside Vellum; the manifest now follows it.
    Renamed {
        /// The section.
        id: NodeId,
        /// Where the manifest last saw it.
        from: ChapterPath,
        /// Where it is now.
        to: ChapterPath,
    },
    /// The summary lists a file the manifest did not know; it was added.
    Added {
        /// The section.
        id: NodeId,
        /// Its file.
        path: ChapterPath,
    },
    /// A file had no id in its front matter; one was written.
    IdAssigned {
        /// The new or manifest-provided id.
        id: NodeId,
        /// The file.
        path: ChapterPath,
    },
    /// Two files carried the same id (a copy made outside Vellum); the
    /// second one got a fresh id.
    Reidentified {
        /// The duplicated id.
        old: NodeId,
        /// The fresh id.
        new: NodeId,
        /// The file that was re-identified.
        path: ChapterPath,
    },
    /// The summary links to a file that does not exist; it was created as
    /// a draft page (R-BK-7).
    CreatedMissing {
        /// The new section.
        id: NodeId,
        /// The created file.
        path: ChapterPath,
    },
    /// The manifest listed a section the summary no longer has.
    Dropped {
        /// The section.
        id: NodeId,
        /// Its last known file.
        path: ChapterPath,
    },
    /// The summary nests deeper than `max_depth`; nothing was changed, but
    /// further nesting is refused until it is fixed.
    TooDeep {
        /// The section that is too deep.
        path: ChapterPath,
        /// Its depth.
        depth: usize,
    },
}

/// A book: `book.yaml`, `SUMMARY.md` and the chapter files they list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub(crate) manifest: Manifest,
    pub(crate) summary: Summary,
    pub(crate) files: BTreeMap<ChapterPath, String>,
    /// Old locations of files that moved; safe to delete once the new
    /// location is written.
    pub(crate) moved_away: BTreeSet<ChapterPath>,
    /// Files of removed sections; to be moved to the drive's trash.
    pub(crate) removed: BTreeSet<ChapterPath>,
}

impl Book {
    /// A new, empty book.
    #[must_use]
    pub fn new(id: BookId, title: &str) -> Self {
        Self {
            manifest: Manifest::new(id, title.to_owned()),
            summary: Summary {
                title: Some("Summary".to_owned()),
                ..Summary::default()
            },
            files: BTreeMap::new(),
            moved_away: BTreeSet::new(),
            removed: BTreeSet::new(),
        }
    }

    /// Loads a book from the text of `book.yaml` and `SUMMARY.md`, reading
    /// each chapter through `read` (which returns `None` for a missing
    /// file), and repairs drift caused by edits made outside Vellum.
    ///
    /// # Errors
    ///
    /// Returns an error when `book.yaml` or `SUMMARY.md` is malformed, or
    /// when the summary lists the same file twice.
    pub fn load(
        manifest: &str,
        summary: &str,
        mut read: impl FnMut(&ChapterPath) -> Option<String>,
        ids: &mut dyn IdSource,
    ) -> Result<(Self, Vec<Repair>)> {
        let manifest = Manifest::parse(manifest)?;
        let summary = Summary::parse(summary)?;
        let mut repairs = Vec::new();
        let mut nodes: BTreeMap<NodeId, NodeEntry> = BTreeMap::new();
        let mut files = BTreeMap::new();
        for (slot, chapter) in summary.chapters() {
            let Some(path) = chapter.location.clone() else {
                continue;
            };
            if files.contains_key(&path) {
                return Err(BookError::Inconsistent(format!(
                    "SUMMARY.md lists {path} more than once"
                )));
            }
            if slot.depth() > usize::from(manifest.max_depth) {
                repairs.push(Repair::TooDeep {
                    path: path.clone(),
                    depth: slot.depth(),
                });
            }
            let (id, text) = identify(
                &manifest,
                &nodes,
                &path,
                &chapter.name,
                read(&path),
                ids,
                &mut repairs,
            );
            let entry = if let Some(known) = manifest.nodes.get(&id) {
                if known.path != path {
                    repairs.push(Repair::Renamed {
                        id: id.clone(),
                        from: known.path.clone(),
                        to: path.clone(),
                    });
                }
                NodeEntry {
                    path: path.clone(),
                    ..known.clone()
                }
            } else {
                let reported = repairs.iter().any(|r| {
                    matches!(r, Repair::CreatedMissing { id: created, .. }
                        | Repair::IdAssigned {
                            id: created,
                            ..
                        } if created == &id)
                });
                if !reported {
                    repairs.push(Repair::Added {
                        id: id.clone(),
                        path: path.clone(),
                    });
                }
                NodeEntry::new(path.clone())
            };
            let _ = nodes.insert(id, entry);
            let _ = files.insert(path, text);
        }
        for (id, entry) in &manifest.nodes {
            if !nodes.contains_key(id) {
                repairs.push(Repair::Dropped {
                    id: id.clone(),
                    path: entry.path.clone(),
                });
            }
        }
        let book = Self {
            manifest: Manifest { nodes, ..manifest },
            summary,
            files,
            moved_away: BTreeSet::new(),
            removed: BTreeSet::new(),
        };
        Ok((book, repairs))
    }

    /// The book's id.
    #[must_use]
    pub const fn id(&self) -> &BookId {
        &self.manifest.id
    }

    /// The book's title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.manifest.title
    }

    /// The parsed `book.yaml`.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The parsed `SUMMARY.md`.
    #[must_use]
    pub const fn summary(&self) -> &Summary {
        &self.summary
    }

    /// The text of `book.yaml`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Manifest`] if serialisation fails.
    pub fn manifest_text(&self) -> Result<String> {
        self.manifest.to_yaml()
    }

    /// The text of `SUMMARY.md`.
    #[must_use]
    pub fn summary_text(&self) -> String {
        self.summary.to_markdown()
    }

    /// Every chapter file and its text.
    #[must_use]
    pub const fn files(&self) -> &BTreeMap<ChapterPath, String> {
        &self.files
    }

    /// Paths whose content moved elsewhere in this session; safe to delete
    /// once the new files are written.
    #[must_use]
    pub const fn moved_away(&self) -> &BTreeSet<ChapterPath> {
        &self.moved_away
    }

    /// Files of sections removed in this session; they belong in the
    /// drive's trash, not deleted outright.
    #[must_use]
    pub const fn removed(&self) -> &BTreeSet<ChapterPath> {
        &self.removed
    }

    /// Forgets [`Self::moved_away`] and [`Self::removed`] after a save.
    pub fn mark_saved(&mut self) {
        self.moved_away.clear();
        self.removed.clear();
    }

    /// Changes how sections are numbered.
    pub const fn set_numbering(
        &mut self,
        style: crate::manifest::NumberingStyle,
    ) {
        self.manifest.numbering = style;
    }

    /// Changes the maximum nesting depth.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Manifest`] outside 1 to 6 and
    /// [`BookError::TooDeep`] when the book already nests deeper.
    pub fn set_max_depth(&mut self, max_depth: u8) -> Result<()> {
        if !(1..=crate::manifest::LIMIT_MAX_DEPTH).contains(&max_depth) {
            return Err(BookError::Manifest(format!(
                "max_depth must be 1 to {}",
                crate::manifest::LIMIT_MAX_DEPTH
            )));
        }
        let deepest = self
            .summary
            .chapters()
            .iter()
            .map(|(slot, _)| slot.depth())
            .max()
            .unwrap_or(0);
        if deepest > usize::from(max_depth) {
            return Err(BookError::TooDeep {
                max: max_depth,
                needed: deepest,
            });
        }
        self.manifest.max_depth = max_depth;
        Ok(())
    }

    /// The id of the section stored at `path`.
    #[must_use]
    pub fn id_at(&self, path: &ChapterPath) -> Option<&NodeId> {
        self.manifest.id_at(path)
    }

    /// The file of section `id`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn path_of(&self, id: &NodeId) -> Result<&ChapterPath> {
        self.manifest
            .nodes
            .get(id)
            .map(|entry| &entry.path)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))
    }

    /// Whether the book contains section `id`.
    #[must_use]
    pub fn contains(&self, id: &NodeId) -> bool {
        self.manifest.nodes.contains_key(id)
    }

    /// Where section `id` sits in the summary.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn slot_of(&self, id: &NodeId) -> Result<Slot> {
        let path = self.path_of(id)?;
        self.summary
            .find(path)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))
    }

    /// The id of the chapter at `slot` (`None` for a draft).
    #[must_use]
    pub fn id_of_slot(&self, slot: &Slot) -> Option<&NodeId> {
        let chapter = self.summary.chapter(slot)?;
        self.id_at(chapter.location.as_ref()?)
    }

    /// The text of section `id`'s file.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn text(&self, id: &NodeId) -> Result<&str> {
        let path = self.path_of(id)?;
        self.files
            .get(path)
            .map(String::as_str)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))
    }

    /// Replaces the text of section `id`'s file (an editor save).
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] for an unknown section and
    /// [`BookError::NotAllowed`] when the new text changes or drops the
    /// front-matter id, which is immutable.
    pub fn set_text(&mut self, id: &NodeId, text: String) -> Result<()> {
        let declared = markdown::front_matter(&text, "id")
            .and_then(|raw| NodeId::parse(&raw).ok());
        if declared.as_ref() != Some(id) {
            return Err(BookError::NotAllowed(format!(
                "the front matter of {id} must keep `id: {id}`"
            )));
        }
        let path = self.path_of(id)?.clone();
        let _ = self.files.insert(path, text);
        Ok(())
    }

    /// The section numbers of every numbered chapter, keyed by slot.
    #[must_use]
    pub fn numbers(&self) -> BTreeMap<Slot, SectionNumber> {
        fn visit(
            slot: &Slot,
            chapter: &Chapter,
            prefix: &[u32],
            out: &mut BTreeMap<Slot, SectionNumber>,
        ) {
            for (index, child) in chapter.children.iter().enumerate() {
                let mut number = prefix.to_vec();
                number.push(u32::try_from(index + 1).unwrap_or(u32::MAX));
                let child_slot = slot.child(index);
                visit(&child_slot, child, &number, out);
                let _ = out.insert(child_slot, SectionNumber(number));
            }
        }
        let mut out = BTreeMap::new();
        let mut counter = 0_u32;
        for (index, item) in self.summary.numbered.iter().enumerate() {
            if let Item::Chapter(chapter) = item {
                counter = counter.saturating_add(1);
                let slot = Slot::top(Section::Numbered, index);
                visit(&slot, chapter, &[counter], &mut out);
                let _ = out.insert(slot, SectionNumber(vec![counter]));
            }
        }
        out
    }

    /// The number of section `id`, `None` when it is unnumbered.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn number_of(&self, id: &NodeId) -> Result<Option<SectionNumber>> {
        let slot = self.slot_of(id)?;
        Ok(self.numbers().remove(&slot))
    }

    /// The navigation title of section `id`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn title_of(&self, id: &NodeId) -> Result<&str> {
        let slot = self.slot_of(id)?;
        self.summary
            .chapter(&slot)
            .map(|c| c.name.as_str())
            .ok_or_else(|| BookError::UnknownNode(id.clone()))
    }

    /// The cross-reference label of section `id`, such as `§2.3 Services`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn label_of(&self, id: &NodeId) -> Result<String> {
        let number = self.number_of(id)?;
        Ok(numbering::label(
            number.as_ref(),
            self.manifest.numbering,
            self.title_of(id)?,
        ))
    }

    /// Ids of section `id` and every section nested under it, in reading
    /// order.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] when the book has no such section.
    pub fn subtree_ids(&self, id: &NodeId) -> Result<Vec<NodeId>> {
        let slot = self.slot_of(id)?;
        Ok(self.ids_under(&slot))
    }

    /// Ids of the chapter at `slot` and its descendants (drafts skipped).
    #[must_use]
    pub fn ids_under(&self, slot: &Slot) -> Vec<NodeId> {
        let mut out = Vec::new();
        if let Some(chapter) = self.summary.chapter(slot) {
            chapter.walk(&mut |c| {
                if let Some(id) =
                    c.location.as_ref().and_then(|p| self.id_at(p))
                {
                    out.push(id.clone());
                }
            });
        }
        out
    }

    /// The slots of the enclosing chapters of `slot`, outermost first.
    #[must_use]
    pub fn ancestors(slot: &Slot) -> Vec<Slot> {
        let mut out = Vec::new();
        let mut current = slot.parent();
        while let Some(parent) = current {
            current = parent.parent();
            out.push(parent);
        }
        out.reverse();
        out
    }
}

/// Works out the id of the chapter at `path` and its (possibly repaired)
/// text.
fn identify(
    manifest: &Manifest,
    seen: &BTreeMap<NodeId, NodeEntry>,
    path: &ChapterPath,
    name: &str,
    text: Option<String>,
    ids: &mut dyn IdSource,
    repairs: &mut Vec<Repair>,
) -> (NodeId, String) {
    let Some(text) = text else {
        let id = manifest
            .id_at(path)
            .filter(|id| !seen.contains_key(*id))
            .cloned()
            .unwrap_or_else(|| ids.next_id());
        repairs.push(Repair::CreatedMissing {
            id: id.clone(),
            path: path.clone(),
        });
        let text = markdown::new_chapter(&id, name, Some("draft"), "");
        return (id, text);
    };
    let declared = markdown::front_matter(&text, "id")
        .and_then(|raw| NodeId::parse(&raw).ok());
    match declared {
        Some(id) if !seen.contains_key(&id) => (id, text),
        Some(old) => {
            let new = ids.next_id();
            repairs.push(Repair::Reidentified {
                old,
                new: new.clone(),
                path: path.clone(),
            });
            let text = markdown::set_front_matter(&text, "id", new.as_str());
            (new, text)
        }
        None => {
            let id = manifest
                .id_at(path)
                .filter(|id| !seen.contains_key(*id))
                .cloned()
                .unwrap_or_else(|| ids.next_id());
            repairs.push(Repair::IdAssigned {
                id: id.clone(),
                path: path.clone(),
            });
            let mut text = markdown::set_front_matter(&text, "id", id.as_str());
            if markdown::front_matter(&text, "title").is_none() {
                text = markdown::set_front_matter(&text, "title", name);
            }
            if markdown::front_matter(&text, "vellum").is_none() {
                text = markdown::set_front_matter(&text, "vellum", "1");
            }
            (id, text)
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::fmt::Write as _;

    use super::*;
    use crate::id::SequentialIds;

    /// The Northstar handbook from the PRD, with ids A1.. in reading order.
    pub const SUMMARY: &str = "# Summary

[Introduction](README.md)

# Part I · Joining

- [Onboarding](onboarding/README.md)
  - [Your first week](onboarding/first-week.md)
  - [Security training]()

# Part II · How it works

- [Architecture](architecture/README.md)
  - [Services](architecture/services.md)

---

[Glossary](glossary.md)
";

    pub fn id(n: u8) -> NodeId {
        NodeId::parse(&format!("01JBK0000000000000000000A{n}"))
            .unwrap_or_else(|_| SequentialIds::default().next_id())
    }

    pub fn handbook() -> Book {
        let paths = [
            ("README.md", 1, "Introduction", "Welcome to Northstar."),
            ("onboarding/README.md", 2, "Onboarding", "How we join."),
            (
                "onboarding/first-week.md",
                3,
                "Your first week",
                "Read [§](vellum://doc/01JBK0000000000000000000A5) first.\n\n\
                    ## Day one\n\nBadge.\n\n## Day two\n\nLaptop.",
            ),
            (
                "architecture/README.md",
                4,
                "Architecture",
                "The big picture.",
            ),
            (
                "architecture/services.md",
                5,
                "Services",
                "See [the glossary](../glossary.md) and \
                    [](vellum://doc/01JBK0000000000000000000A2).",
            ),
            ("glossary.md", 6, "Glossary", "Terms."),
        ];
        let mut manifest = String::from(
            "vellum: 1\nid: 01JBK00000000000000000B00K\n\
                title: Northstar handbook\nnodes:\n",
        );
        let mut files = BTreeMap::new();
        for (path, n, title, body) in paths {
            let _ = write!(manifest, "  {}:\n    path: {path}\n", id(n));
            let _ = files.insert(
                path.to_owned(),
                markdown::new_chapter(&id(n), title, None, body),
            );
        }
        let mut ids = SequentialIds::new("01JBK00009");
        match Book::load(
            &manifest,
            SUMMARY,
            |p| files.get(p.as_str()).cloned(),
            &mut ids,
        ) {
            Ok((book, _)) => book,
            Err(e) => unreachable!("fixture must load: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{handbook, id};
    use super::*;
    use crate::id::SequentialIds;

    #[test]
    fn numbers_follow_mdbook() -> Result<()> {
        let book = handbook();
        assert_eq!(book.number_of(&id(1))?, None);
        assert_eq!(book.label_of(&id(2))?, "§1 Onboarding");
        assert_eq!(book.label_of(&id(3))?, "§1.1 Your first week");
        assert_eq!(book.label_of(&id(5))?, "§2.1 Services");
        assert_eq!(book.label_of(&id(6))?, "Glossary");
        // The draft counts: it is 1.2.
        let draft = Slot {
            section: Section::Numbered,
            path: vec![1, 1],
        };
        assert_eq!(
            book.numbers().get(&draft),
            Some(&SectionNumber(vec![1, 2]))
        );
        Ok(())
    }

    #[test]
    fn load_repairs_external_edits() -> Result<()> {
        let book = handbook();
        let manifest = book.manifest_text()?;
        // Rename services.md outside Vellum, add an id-less file, and
        // reference a file that does not exist.
        let summary = book
            .summary_text()
            .replace("architecture/services.md", "architecture/svc.md")
            .replace(
                "[Glossary](glossary.md)",
                "[Glossary](glossary.md)\n\n[Notes](notes.md)\n\n\
                    [Later](later.md)",
            );
        let mut files: BTreeMap<String, String> = book
            .files()
            .iter()
            .map(|(p, t)| {
                (p.as_str().replace("services.md", "svc.md"), t.clone())
            })
            .collect();
        let _ = files.insert("notes.md".into(), "Just notes.\n".into());
        let mut ids = SequentialIds::new("01JBK00008");
        let (loaded, repairs) = Book::load(
            &manifest,
            &summary,
            |p| files.get(p.as_str()).cloned(),
            &mut ids,
        )?;
        assert!(repairs.contains(&Repair::Renamed {
            id: id(5),
            from: ChapterPath::parse("architecture/services.md")?,
            to: ChapterPath::parse("architecture/svc.md")?,
        }));
        assert!(repairs.iter().any(|r| matches!(
            r,
            Repair::IdAssigned {
                path,
                ..
            } if path.as_str() == "notes.md"
        )));
        let created = repairs.iter().find_map(|r| match r {
            Repair::CreatedMissing { id, path }
                if path.as_str() == "later.md" =>
            {
                Some(id.clone())
            }
            _ => None,
        });
        let created = created
            .ok_or_else(|| BookError::Inconsistent("no draft".into()))?;
        assert_eq!(
            markdown::front_matter(loaded.text(&created)?, "status").as_deref(),
            Some("draft")
        );
        let notes = loaded
            .id_at(&ChapterPath::parse("notes.md")?)
            .cloned()
            .ok_or_else(|| BookError::Inconsistent("notes".into()))?;
        assert_eq!(
            markdown::front_matter(loaded.text(&notes)?, "title").as_deref(),
            Some("Notes")
        );
        Ok(())
    }

    #[test]
    fn duplicate_ids_are_reissued() -> Result<()> {
        let book = handbook();
        let summary = book.summary_text().replace(
            "[Glossary](glossary.md)",
            "[Glossary](glossary.md)\n\n[Copy](copy.md)",
        );
        let mut files: BTreeMap<String, String> = book
            .files()
            .iter()
            .map(|(p, t)| (p.as_str().to_owned(), t.clone()))
            .collect();
        let glossary = files.get("glossary.md").cloned().unwrap_or_default();
        let _ = files.insert("copy.md".into(), glossary);
        let mut ids = SequentialIds::new("01JBK00008");
        let (_, repairs) = Book::load(
            &book.manifest_text()?,
            &summary,
            |p| files.get(p.as_str()).cloned(),
            &mut ids,
        )?;
        assert!(repairs.iter().any(
            |r| matches!(r, Repair::Reidentified { old, .. } if old == &id(6))
        ));
        Ok(())
    }

    #[test]
    fn ids_are_immutable() {
        let mut book = handbook();
        let result = book.set_text(
            &id(6),
            "---\nid: 01JBK0000000000000000000A7\n---\n".into(),
        );
        assert!(result.is_err());
        let ok = markdown::new_chapter(&id(6), "Glossary", None, "More terms.");
        assert!(book.set_text(&id(6), ok).is_ok());
    }

    #[test]
    fn rejects_duplicate_summary_paths() -> Result<()> {
        let book = handbook();
        let summary =
            format!("{}\n[Again](glossary.md)\n", book.summary_text());
        let files = book.files().clone();
        let result = Book::load(
            &book.manifest_text()?,
            &summary,
            |p| files.get(p).cloned(),
            &mut SequentialIds::default(),
        );
        assert!(result.is_err());
        Ok(())
    }
}
