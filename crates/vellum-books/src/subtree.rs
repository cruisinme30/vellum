// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Moving and copying subtrees within and between books, with link
//! rewriting (R-BK-6).
//!
//! - **Move** keeps every id, so `vellum://doc/<id>` links anywhere keep
//!   working. Relative links that would cross the book boundary are turned
//!   into id links on both sides.
//! - **Copy** gives every copied section a fresh id (R-DR-3: copying
//!   re-identifies every node) and points links inside the copy at the
//!   copies; links to sections outside the copy still point at the
//!   originals.
//!
//! Non-chapter files the subtree links to (images, attachments) are listed
//! in [`Subtree::assets`]; the caller copies them to the same relative path
//! in the target book, and the links are rewritten to match.

use std::collections::{BTreeMap, BTreeSet};

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::{BookId, IdSource, NodeId};
use crate::manifest::NodeEntry;
use crate::markdown::{self, LinkEdit, LinkKind};
use crate::ops::{self, NodeRef, Place};
use crate::path::{self, ChapterPath};
use crate::summary::{Chapter, Item, Slot};

/// A detached subtree: chapters, their settings and their files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subtree {
    /// The book it came from.
    pub source: BookId,
    /// The root chapter and its descendants; locations are paths in the
    /// source book.
    pub root: Chapter,
    /// Per-section settings keyed by id; paths are paths in the source book.
    pub entries: BTreeMap<NodeId, NodeEntry>,
    /// File texts keyed by their path in the source book.
    pub files: BTreeMap<ChapterPath, String>,
    /// Non-chapter files the subtree links to, relative to the source
    /// book folder. The caller copies them to the same path in the target.
    pub assets: BTreeSet<String>,
}

impl Subtree {
    /// The ids of the sections in the subtree.
    #[must_use]
    pub fn ids(&self) -> Vec<NodeId> {
        self.entries.keys().cloned().collect()
    }

    /// Gives every section a fresh id and points id links inside the
    /// subtree at the new ids. Returns `old → new`.
    pub fn reidentify(
        &mut self,
        ids: &mut dyn IdSource,
    ) -> BTreeMap<NodeId, NodeId> {
        let map: BTreeMap<NodeId, NodeId> = self
            .entries
            .keys()
            .map(|old| (old.clone(), ids.next_id()))
            .collect();
        self.entries = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(old, entry)| (map.get(&old).cloned().unwrap_or(old), entry))
            .collect();
        for (new, entry) in &self.entries {
            if let Some(text) = self.files.get_mut(&entry.path) {
                let with_id =
                    markdown::set_front_matter(text, "id", new.as_str());
                *text = retarget_ids(&with_id, &map);
            }
        }
        map
    }
}

/// Rewrites `vellum://doc/<old>` links to `<new>` for every id in `map`.
pub(crate) fn retarget_ids(
    text: &str,
    map: &BTreeMap<NodeId, NodeId>,
) -> String {
    markdown::map_links(text, |link, _| {
        let (id, fragment) = NodeId::from_uri(&link.dest)?;
        let new = map.get(&id)?;
        Some(LinkEdit {
            dest: Some(with_fragment(&new.uri(), fragment)),
            ..LinkEdit::default()
        })
    })
}

fn with_fragment(dest: &str, fragment: Option<&str>) -> String {
    fragment.map_or_else(|| dest.to_owned(), |f| format!("{dest}#{f}"))
}

fn split_fragment(dest: &str) -> (&str, Option<&str>) {
    match dest.split_once('#') {
        Some((target, fragment)) => (target, Some(fragment)),
        None => (dest, None),
    }
}

impl Book {
    /// Copies a section and everything under it out of the book, without
    /// changing the book.
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist.
    pub fn extract(&self, node: &NodeRef) -> Result<Subtree> {
        let slot = self.resolve(node)?;
        let root = self
            .summary
            .chapter(&slot)
            .cloned()
            .ok_or_else(|| BookError::UnknownPosition(slot.to_string()))?;
        let inside: BTreeSet<NodeId> =
            self.ids_under(&slot).into_iter().collect();
        let chapter_ids: BTreeMap<&str, &NodeId> = self
            .manifest
            .nodes
            .iter()
            .map(|(id, entry)| (entry.path.as_str(), id))
            .collect();
        let mut entries = BTreeMap::new();
        let mut files = BTreeMap::new();
        let mut assets = BTreeSet::new();
        for id in &inside {
            let entry = self
                .manifest
                .nodes
                .get(id)
                .cloned()
                .ok_or_else(|| BookError::UnknownNode(id.clone()))?;
            let text = self.text(id)?;
            let dir = entry.path.dir().to_owned();
            // Relative links that leave the subtree become id links.
            let text = markdown::map_links(text, |link, _| {
                if link.kind == LinkKind::Autolink {
                    return None;
                }
                let (target, fragment) = split_fragment(&link.dest);
                let resolved = path::resolve(&dir, target)?;
                match chapter_ids.get(resolved.as_str()) {
                    Some(target_id) if !inside.contains(*target_id) => {
                        Some(LinkEdit {
                            dest: Some(with_fragment(
                                &target_id.uri(),
                                fragment,
                            )),
                            ..LinkEdit::default()
                        })
                    }
                    Some(_) => None,
                    None => {
                        let _ = assets.insert(resolved);
                        None
                    }
                }
            });
            let _ = files.insert(entry.path.clone(), text);
            let _ = entries.insert(id.clone(), entry);
        }
        Ok(Subtree {
            source: self.manifest.id.clone(),
            root,
            entries,
            files,
            assets,
        })
    }

    /// Removes a section and everything under it from the book, for moving
    /// it elsewhere. Relative links from the rest of the book into it
    /// become id links. Its old files are listed in [`Book::moved_away`]:
    /// write the destination before deleting them.
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist.
    pub fn take(&mut self, node: &NodeRef) -> Result<Subtree> {
        let subtree = self.extract(node)?;
        let slot = self.resolve(node)?;
        let parent = slot.parent().and_then(|p| self.id_of_slot(&p).cloned());
        let _ = self.summary.remove(&slot)?;
        let leaving: BTreeMap<String, NodeId> = subtree
            .entries
            .iter()
            .map(|(id, entry)| (entry.path.as_str().to_owned(), id.clone()))
            .collect();
        for (id, entry) in &subtree.entries {
            let _ = self.manifest.nodes.remove(id);
            let _ = self.files.remove(&entry.path);
            let _ = self.moved_away.insert(entry.path.clone());
        }
        for (path, text) in &mut self.files {
            let dir = path.dir().to_owned();
            *text = markdown::map_links(text, |link, _| {
                if link.kind == LinkKind::Autolink {
                    return None;
                }
                let (target, fragment) = split_fragment(&link.dest);
                let resolved = path::resolve(&dir, target)?;
                let id = leaving.get(&resolved)?;
                Some(LinkEdit {
                    dest: Some(with_fragment(&id.uri(), fragment)),
                    ..LinkEdit::default()
                })
            });
        }
        if let Some(parent) = parent {
            let parent_slot = self.slot_of(&parent)?;
            self.relayout(&[parent_slot], &BTreeMap::new())?;
        }
        Ok(subtree)
    }

    /// Inserts a detached subtree at `place`, giving its files paths that
    /// mirror the tree and fixing relative links. Returns where it landed.
    ///
    /// # Errors
    ///
    /// Returns an error when the place does not exist or is not allowed,
    /// when the subtree would nest deeper than `max_depth`, or when one of
    /// its ids is already in this book (copy with fresh ids instead).
    pub fn graft(
        &mut self,
        mut subtree: Subtree,
        place: &Place,
    ) -> Result<Slot> {
        if let Some(id) = subtree.entries.keys().find(|id| self.contains(id)) {
            return Err(BookError::NotAllowed(format!(
                "section {id} is already in this book; copy it with fresh ids \
                    instead"
            )));
        }
        let dest = self.place(place)?;
        let needed = dest.depth() + subtree.root.height() - 1;
        if dest.section == crate::summary::Section::Numbered
            && needed > usize::from(self.manifest.max_depth)
        {
            return Err(BookError::TooDeep {
                max: self.manifest.max_depth,
                needed,
            });
        }
        // Park every file at a unique temporary path at the book root, with
        // its relative links valid for that location; relayout then moves
        // them where the tree says.
        let mut taken: BTreeSet<String> =
            self.files.keys().map(|p| p.as_str().to_owned()).collect();
        let mut parked = BTreeMap::new();
        for source in subtree.files.keys() {
            let stem = source.file_name().trim_end_matches(".md");
            let stem = if source.is_readme() {
                source.dir().rsplit('/').next().unwrap_or(stem)
            } else {
                stem
            };
            let temp = ops::unique(
                "",
                &format!("{}.md", path::slugify(stem)),
                &taken,
            )?;
            let _ = taken.insert(temp.as_str().to_owned());
            let _ = parked.insert(source.clone(), temp);
        }
        let sources: BTreeSet<String> = subtree
            .files
            .keys()
            .map(|p| p.as_str().to_owned())
            .collect();
        let mut summary = self.summary.clone();
        relocate_chapter(&mut subtree.root, &parked);
        summary.insert(&dest, Item::Chapter(subtree.root))?;
        self.summary = summary;
        for (source, text) in subtree.files {
            let Some(temp) = parked.get(&source) else {
                continue;
            };
            let edits = ops::rewrite_relative(
                &text,
                source.dir(),
                "",
                &parked,
                &sources,
            );
            let _ = self
                .files
                .insert(temp.clone(), markdown::apply(&text, edits));
        }
        for (id, mut entry) in subtree.entries {
            if let Some(temp) = parked.get(&entry.path) {
                entry.path = temp.clone();
            }
            let _ = self.manifest.nodes.insert(id, entry);
        }
        let mut roots = vec![dest.clone()];
        roots.extend(dest.parent());
        self.relayout(&roots, &BTreeMap::new())?;
        Ok(dest)
    }

    /// Copies a section and its subtree to `place` in the same book, with
    /// fresh ids. Returns `old id → new id`.
    ///
    /// # Errors
    ///
    /// As for [`Book::extract`] and [`Book::graft`].
    pub fn copy_section(
        &mut self,
        node: &NodeRef,
        place: &Place,
        ids: &mut dyn IdSource,
    ) -> Result<BTreeMap<NodeId, NodeId>> {
        let mut subtree = self.extract(node)?;
        let map = subtree.reidentify(ids);
        let _ = self.graft(subtree, place)?;
        Ok(map)
    }
}

fn relocate_chapter(
    chapter: &mut Chapter,
    map: &BTreeMap<ChapterPath, ChapterPath>,
) {
    if let Some(new) = chapter.location.as_ref().and_then(|p| map.get(p)) {
        chapter.location = Some(new.clone());
    }
    for child in &mut chapter.children {
        relocate_chapter(child, map);
    }
}

/// What a move or copy between books produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// Where the subtree landed in the target book.
    pub slot: Slot,
    /// `old id → new id` for a copy; empty for a move.
    pub ids: BTreeMap<NodeId, NodeId>,
    /// Files the caller must copy from the source book folder to the same
    /// relative path in the target book folder.
    pub assets: BTreeSet<String>,
}

/// Moves a section and its subtree from one book to another, keeping ids.
///
/// # Errors
///
/// As for [`Book::take`] and [`Book::graft`]; on error neither book is
/// changed.
pub fn move_between(
    source: &mut Book,
    node: &NodeRef,
    target: &mut Book,
    place: &Place,
) -> Result<Transfer> {
    let mut trial_source = source.clone();
    let subtree = trial_source.take(node)?;
    let assets = subtree.assets.clone();
    let mut trial_target = target.clone();
    let slot = trial_target.graft(subtree, place)?;
    *source = trial_source;
    *target = trial_target;
    Ok(Transfer {
        slot,
        ids: BTreeMap::new(),
        assets,
    })
}

/// Copies a section and its subtree into another book with fresh ids.
///
/// # Errors
///
/// As for [`Book::extract`] and [`Book::graft`]; on error the target is
/// not changed.
pub fn copy_between(
    source: &Book,
    node: &NodeRef,
    target: &mut Book,
    place: &Place,
    ids: &mut dyn IdSource,
) -> Result<Transfer> {
    let mut subtree = source.extract(node)?;
    let map = subtree.reidentify(ids);
    let assets = subtree.assets.clone();
    let mut trial = target.clone();
    let slot = trial.graft(subtree, place)?;
    *target = trial;
    Ok(Transfer {
        slot,
        ids: map,
        assets,
    })
}

/// Copies a whole book with fresh ids for the book and every section
/// (R-DR-3).
///
/// # Errors
///
/// Returns an error if the copy cannot be assembled (it never should for a
/// consistent book).
pub fn copy_book(
    source: &Book,
    new_id: BookId,
    title: &str,
    ids: &mut dyn IdSource,
) -> Result<(Book, BTreeMap<NodeId, NodeId>)> {
    let map: BTreeMap<NodeId, NodeId> = source
        .manifest
        .nodes
        .keys()
        .map(|old| (old.clone(), ids.next_id()))
        .collect();
    let mut book = source.clone();
    book.manifest.id = new_id;
    title.clone_into(&mut book.manifest.title);
    book.manifest.nodes = std::mem::take(&mut book.manifest.nodes)
        .into_iter()
        .map(|(old, entry)| (map.get(&old).cloned().unwrap_or(old), entry))
        .collect();
    for (id, entry) in &book.manifest.nodes {
        if let Some(text) = book.files.get_mut(&entry.path) {
            let with_id = markdown::set_front_matter(text, "id", id.as_str());
            *text = retarget_ids(&with_id, &map);
        }
    }
    book.moved_away.clear();
    book.removed.clear();
    Ok((book, map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};
    use crate::id::SequentialIds;
    use crate::summary::Section;

    fn other_book() -> Book {
        let mut book = Book::new(
            BookId::parse("01JBK00000000000000000B01K").unwrap(),
            "Runbooks",
        );
        let mut ids = SequentialIds::new("01JBK00006");
        let _ = book
            .add_section(
                &Place::End(Section::Numbered),
                "On call",
                "Pager rota.",
                &mut ids,
            )
            .unwrap();
        book
    }

    #[test]
    fn move_architecture_to_another_book() {
        let mut source = handbook();
        let mut target = other_book();
        let on_call = target
            .id_at(&ChapterPath::parse("on-call.md").unwrap())
            .unwrap()
            .clone();
        let transfer = move_between(
            &mut source,
            &id(4).into(),
            &mut target,
            &Place::LastChildOf(on_call.into()),
        )
        .unwrap();
        assert!(transfer.ids.is_empty());
        assert!(!source.contains(&id(4)) && !source.contains(&id(5)));
        assert!(target.contains(&id(4)) && target.contains(&id(5)));
        assert_eq!(target.label_of(&id(5)).unwrap(), "§1.1.1 Services");
        assert_eq!(
            target.path_of(&id(5)).unwrap().as_str(),
            "on-call/architecture/services.md"
        );
        // services linked to ../glossary.md, which stayed behind: now an id
        // link.
        let services = target.text(&id(5)).unwrap();
        assert!(
            services.contains(
                "[the glossary](vellum://doc/01JBK0000000000000000000A6)"
            ),
            "{services}"
        );
        assert!(
            source
                .moved_away()
                .iter()
                .any(|p| p.as_str() == "architecture/services.md")
        );
        // The source summary no longer lists them.
        assert!(!source.summary_text().contains("architecture"));
    }

    #[test]
    fn copy_reidentifies_and_retargets_internal_links() {
        let source = handbook();
        let mut target = other_book();
        let mut ids = SequentialIds::new("01JBK00005");
        let transfer = copy_between(
            &source,
            &id(2).into(),
            &mut target,
            &Place::End(Section::Numbered),
            &mut ids,
        )
        .unwrap();
        let new_onboarding = transfer.ids.get(&id(2)).unwrap();
        let new_week = transfer.ids.get(&id(3)).unwrap();
        assert!(target.contains(new_onboarding));
        assert_eq!(
            markdown::front_matter(target.text(new_week).unwrap(), "id")
                .as_deref(),
            Some(new_week.as_str())
        );
        // The week page linked to Services (outside the copy): still the
        // original.
        assert!(
            target
                .text(new_week)
                .unwrap()
                .contains("vellum://doc/01JBK0000000000000000000A5")
        );
        // The source is untouched.
        assert_eq!(source, handbook());
        // The draft child came along.
        assert_eq!(
            target.toc().find(new_onboarding).unwrap().children.len(),
            2
        );
    }

    #[test]
    fn copy_within_a_book() {
        let mut book = handbook();
        let mut ids = SequentialIds::new("01JBK00005");
        let map = book
            .copy_section(&id(4).into(), &Place::After(id(4).into()), &mut ids)
            .unwrap();
        let copy = map.get(&id(4)).unwrap();
        assert_eq!(
            book.path_of(copy).unwrap().as_str(),
            "architecture-2/README.md"
        );
        let services_copy = map.get(&id(5)).unwrap();
        assert!(
            book.text(services_copy)
                .unwrap()
                .contains("(vellum://doc/01JBK0000000000000000000A6)")
        );
        assert_eq!(book.label_of(services_copy).unwrap(), "§3.1 Services");
        // Grafting the same ids twice is refused.
        let subtree = book.extract(&id(4).into()).unwrap();
        assert!(book.graft(subtree, &Place::End(Section::Numbered)).is_err());
    }

    #[test]
    fn copy_a_whole_book() {
        let source = handbook();
        let mut ids = SequentialIds::new("01JBK00005");
        let (copy, map) = copy_book(
            &source,
            BookId::parse("01JBK00000000000000000B02K").unwrap(),
            "Handbook copy",
            &mut ids,
        )
        .unwrap();
        assert_eq!(map.len(), 6);
        let week = map.get(&id(3)).unwrap();
        let services = map.get(&id(5)).unwrap();
        assert!(copy.text(week).unwrap().contains(&services.uri()));
        assert_eq!(copy.label_of(services).unwrap(), "§2.1 Services");
    }
}
