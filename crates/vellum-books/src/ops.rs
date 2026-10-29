// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Editing the tree: add, reorder, nest, rename and remove sections
//! (R-BK-1, R-BK-7).
//!
//! Every edit rewrites `SUMMARY.md` and keeps the folder mirroring the
//! tree: a section with children is `<slug>/README.md` with its children
//! beside it, a leaf is `<slug>.md`. When a file moves, relative links to
//! it and from it are rewritten; `vellum://doc/<id>` links never need to
//! change, which is why they are the canonical form.

use std::collections::{BTreeMap, BTreeSet};

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::{IdSource, NodeId};
use crate::manifest::NodeEntry;
use crate::markdown::{self, LinkKind};
use crate::path::{self, ChapterPath, README};
use crate::summary::{Chapter, Item, Section, Slot};

/// A section, by id or by position (drafts have no id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeRef {
    /// A section with a file.
    Id(NodeId),
    /// Any chapter, including drafts.
    Slot(Slot),
}

impl From<NodeId> for NodeRef {
    fn from(id: NodeId) -> Self {
        Self::Id(id)
    }
}

impl From<&NodeId> for NodeRef {
    fn from(id: &NodeId) -> Self {
        Self::Id(id.clone())
    }
}

impl From<Slot> for NodeRef {
    fn from(slot: Slot) -> Self {
        Self::Slot(slot)
    }
}

/// Where to put a section: what a drag and drop in the navigation says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// Just before a section, as its sibling.
    Before(NodeRef),
    /// Just after a section, as its sibling.
    After(NodeRef),
    /// As the first child of a section (drag onto, top).
    FirstChildOf(NodeRef),
    /// As the last child of a section (drag onto).
    LastChildOf(NodeRef),
    /// At the start of a region.
    Start(Section),
    /// At the end of a region.
    End(Section),
}

impl Book {
    /// The slot a [`NodeRef`] points at.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownNode`] or [`BookError::UnknownPosition`]
    /// when it points at nothing.
    pub fn resolve(&self, node: &NodeRef) -> Result<Slot> {
        match node {
            NodeRef::Id(id) => self.slot_of(id),
            NodeRef::Slot(slot) => {
                if self.summary.chapter(slot).is_some() {
                    Ok(slot.clone())
                } else {
                    Err(BookError::UnknownPosition(slot.to_string()))
                }
            }
        }
    }

    /// The insertion slot a [`Place`] describes, in the current tree.
    ///
    /// # Errors
    ///
    /// Returns an error when the anchor section does not exist.
    pub fn place(&self, place: &Place) -> Result<Slot> {
        Ok(match place {
            Place::Before(node) => self.resolve(node)?,
            Place::After(node) => {
                let mut slot = self.resolve(node)?;
                if let Some(last) = slot.path.last_mut() {
                    *last += 1;
                }
                slot
            }
            Place::FirstChildOf(node) => self.resolve(node)?.child(0),
            Place::LastChildOf(node) => {
                let slot = self.resolve(node)?;
                let count = self.summary.chapter(&slot).map_or(0, |c| c.children.len());
                slot.child(count)
            }
            Place::Start(section) => Slot::top(*section, 0),
            Place::End(section) => Slot::top(*section, self.summary.items(*section).len()),
        })
    }

    fn check_depth(&self, slot: &Slot, height: usize) -> Result<()> {
        let needed = slot.depth() + height - 1;
        let max = self.manifest.max_depth;
        if slot.section == Section::Numbered && needed > usize::from(max) {
            return Err(BookError::TooDeep { max, needed });
        }
        Ok(())
    }

    /// Adds a new section with a file, returning its id.
    ///
    /// # Errors
    ///
    /// Returns an error when the place does not exist, would nest too
    /// deep, or is not allowed by the mdBook structure.
    pub fn add_section(
        &mut self,
        place: &Place,
        title: &str,
        body: &str,
        ids: &mut dyn IdSource,
    ) -> Result<NodeId> {
        let title = clean_title(title)?;
        let slot = self.place(place)?;
        self.check_depth(&slot, 1)?;
        let id = ids.next_id();
        // A placeholder path; relayout gives it its real name.
        let temp = self.unique_path("", &format!("{}.md", path::slugify(&title)))?;
        let mut summary = self.summary.clone();
        summary.insert(&slot, Item::Chapter(Chapter::new(title.clone(), temp.clone())))?;
        self.summary = summary;
        let _ = self.manifest.nodes.insert(id.clone(), NodeEntry::new(temp.clone()));
        let _ = self
            .files
            .insert(temp, markdown::new_chapter(&id, &title, None, body));
        let mut roots = vec![slot.clone()];
        roots.extend(slot.parent());
        self.relayout(&roots, &BTreeMap::new())?;
        Ok(id)
    }

    /// Adds a draft section (`[Title]()`), which has no file yet.
    ///
    /// # Errors
    ///
    /// As for [`Book::add_section`].
    pub fn add_draft(&mut self, place: &Place, title: &str) -> Result<Slot> {
        let title = clean_title(title)?;
        let slot = self.place(place)?;
        self.check_depth(&slot, 1)?;
        self.summary.insert(&slot, Item::Chapter(Chapter::draft(title)))?;
        Ok(slot)
    }

    /// Gives a draft section a file, turning it into a real section.
    ///
    /// # Errors
    ///
    /// Returns an error when `slot` is not a draft.
    pub fn write_draft(&mut self, slot: &Slot, body: &str, ids: &mut dyn IdSource) -> Result<NodeId> {
        let chapter = self
            .summary
            .chapter(slot)
            .ok_or_else(|| BookError::UnknownPosition(slot.to_string()))?;
        if chapter.location.is_some() {
            return Err(BookError::NotAllowed(format!("{slot} is not a draft")));
        }
        let title = chapter.name.clone();
        let id = ids.next_id();
        let temp = self.unique_path("", &format!("{}.md", path::slugify(&title)))?;
        if let Some(chapter) = self.summary.chapter_mut(slot) {
            chapter.location = Some(temp.clone());
        }
        let _ = self.manifest.nodes.insert(id.clone(), NodeEntry::new(temp.clone()));
        let _ = self
            .files
            .insert(temp, markdown::new_chapter(&id, &title, None, body));
        self.relayout(std::slice::from_ref(slot), &BTreeMap::new())?;
        Ok(id)
    }

    /// Adds a part title to the numbered chapters at top-level `index`.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn add_part(&mut self, index: usize, title: &str) -> Result<()> {
        let title = clean_title(title)?;
        self.summary
            .insert(&Slot::top(Section::Numbered, index), Item::Part(title))
    }

    /// Adds a `---` separator.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end.
    pub fn add_separator(&mut self, section: Section, index: usize) -> Result<()> {
        self.summary
            .insert(&Slot::top(section, index), Item::Separator)
    }

    /// Removes a part title or separator.
    ///
    /// # Errors
    ///
    /// Returns an error when the item at `slot` is a chapter or missing.
    pub fn remove_marker(&mut self, slot: &Slot) -> Result<()> {
        match self.summary.item(slot) {
            Some(Item::Part(_) | Item::Separator) if slot.depth() == 1 => {
                let _ = self.summary.remove(slot)?;
                Ok(())
            }
            _ => Err(BookError::NotAllowed(format!(
                "{slot} is not a part title or separator"
            ))),
        }
    }

    /// Moves a section (and everything under it) to `to`: reorder, nest,
    /// un-nest, or move between the prefix, numbered and suffix regions.
    ///
    /// # Errors
    ///
    /// Returns an error when the section or place does not exist, when it
    /// would move into itself, nest deeper than `max_depth`, or nest in the
    /// prefix or suffix region.
    pub fn move_section(&mut self, node: &NodeRef, to: &Place) -> Result<()> {
        let from = self.resolve(node)?;
        let mut dest = self.place(to)?;
        if from.contains(&dest) && dest != from {
            if dest.path.len() == from.path.len() {
                // `After(itself)`: a no-op.
                return Ok(());
            }
            return Err(BookError::NotAllowed(
                "a section cannot move into itself".to_owned(),
            ));
        }
        let height = self.summary.chapter(&from).map_or(1, Chapter::height);
        let old_parent = from.parent().and_then(|p| self.id_of_slot(&p).cloned());
        let mut summary = self.summary.clone();
        let item = summary.remove(&from)?;
        adjust_after_removal(&mut dest, &from);
        self.check_depth(&dest, height)?;
        summary.insert(&dest, item)?;
        self.summary = summary;
        let mut roots = vec![dest.clone()];
        roots.extend(dest.parent());
        if let Some(parent) = old_parent {
            roots.push(self.slot_of(&parent)?);
        }
        self.relayout(&roots, &BTreeMap::new())
    }

    /// Renames a section: its navigation title, its front-matter title and
    /// its file name (relative links follow; id links never break).
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist or the title is
    /// empty.
    pub fn rename(&mut self, node: &NodeRef, title: &str) -> Result<()> {
        let title = clean_title(title)?;
        let slot = self.resolve(node)?;
        let id = self.id_of_slot(&slot).cloned();
        if let Some(chapter) = self.summary.chapter_mut(&slot) {
            chapter.name.clone_from(&title);
        }
        if let Some(id) = id {
            let path = self.path_of(&id)?.clone();
            if let Some(text) = self.files.get_mut(&path) {
                *text = markdown::set_front_matter(text, "title", &title);
            }
            let is_root_intro = slot.depth() == 1 && path.as_str() == README;
            if !is_root_intro {
                let stems = BTreeMap::from([(slot.clone(), path::slugify(&title))]);
                self.relayout(&[slot], &stems)?;
            }
        }
        Ok(())
    }

    /// Removes a section and everything under it, returning the removed
    /// section ids. Their files are listed in [`Book::removed`] for the
    /// caller to move to the trash.
    ///
    /// # Errors
    ///
    /// Returns an error when the section does not exist.
    pub fn remove_section(&mut self, node: &NodeRef) -> Result<Vec<NodeId>> {
        let slot = self.resolve(node)?;
        let ids = self.ids_under(&slot);
        let parent = slot.parent().and_then(|p| self.id_of_slot(&p).cloned());
        let _ = self.summary.remove(&slot)?;
        for id in &ids {
            if let Some(entry) = self.manifest.nodes.remove(id) {
                let _ = self.files.remove(&entry.path);
                let _ = self.removed.insert(entry.path);
            }
        }
        if let Some(parent) = parent {
            let parent_slot = self.slot_of(&parent)?;
            self.relayout(&[parent_slot], &BTreeMap::new())?;
        }
        Ok(ids)
    }

    /// A path that no chapter uses yet, adding `-2`, `-3`… to the stem.
    pub(crate) fn unique_path(&self, dir: &str, file: &str) -> Result<ChapterPath> {
        let taken: BTreeSet<String> = self.files.keys().map(|p| p.as_str().to_owned()).collect();
        unique(dir, file, &taken)
    }

    /// Moves files so the folder mirrors the tree under each root, then
    /// rewrites links. `stems` forces a file name for some slots (rename).
    pub(crate) fn relayout(&mut self, roots: &[Slot], stems: &BTreeMap<Slot, String>) -> Result<()> {
        let mut roots: Vec<Slot> = roots
            .iter()
            .filter(|s| self.summary.chapter(s).is_some())
            .cloned()
            .collect();
        roots.sort_by_key(Slot::depth);
        let mut planned: Vec<Slot> = Vec::new();
        for root in roots {
            if !planned.iter().any(|p| p.contains(&root)) {
                planned.push(root);
            }
        }
        let mut taken: BTreeSet<String> =
            self.files.keys().map(|p| p.as_str().to_owned()).collect();
        let mut assigned: BTreeMap<Slot, ChapterPath> = BTreeMap::new();
        for root in &planned {
            self.plan(root, stems, &mut taken, &mut assigned)?;
        }
        let mut moves = BTreeMap::new();
        for (slot, target) in assigned {
            if let Some(current) = self.summary.chapter(&slot).and_then(|c| c.location.clone())
                && current != target
            {
                let _ = moves.insert(current, target);
            }
        }
        self.relocate(&moves);
        Ok(())
    }

    fn children_dir(&self, slot: &Slot, assigned: &BTreeMap<Slot, ChapterPath>) -> String {
        let Some(chapter) = self.summary.chapter(slot) else {
            return String::new();
        };
        assigned.get(slot).or(chapter.location.as_ref()).map_or_else(
            || join_dir(&self.parent_dir(slot, assigned), &path::slugify(&chapter.name)),
            ChapterPath::children_dir,
        )
    }

    fn parent_dir(&self, slot: &Slot, assigned: &BTreeMap<Slot, ChapterPath>) -> String {
        slot.parent()
            .map(|parent| self.children_dir(&parent, assigned))
            .unwrap_or_default()
    }

    fn plan(
        &self,
        slot: &Slot,
        stems: &BTreeMap<Slot, String>,
        taken: &mut BTreeSet<String>,
        assigned: &mut BTreeMap<Slot, ChapterPath>,
    ) -> Result<()> {
        let Some(chapter) = self.summary.chapter(slot) else {
            return Ok(());
        };
        if let Some(current) = &chapter.location {
            let dir = self.parent_dir(slot, assigned);
            let has_children = !chapter.children.is_empty();
            let root_intro = slot.depth() == 1 && current.as_str() == README && !has_children;
            let stem = stems.get(slot).cloned().unwrap_or_else(|| {
                if current.is_readme() {
                    let own_dir = current.dir();
                    let last = own_dir.rsplit('/').next().unwrap_or_default();
                    if last.is_empty() {
                        path::slugify(&chapter.name)
                    } else {
                        last.to_owned()
                    }
                } else {
                    current.file_name().trim_end_matches(".md").to_owned()
                }
            });
            let target = if root_intro {
                current.clone()
            } else if has_children {
                ChapterPath::join(&join_dir(&dir, &stem), README)?
            } else {
                ChapterPath::join(&dir, &format!("{stem}.md"))?
            };
            let target = if &target == current {
                target
            } else if has_children {
                let mut own = BTreeSet::new();
                chapter.walk(&mut |c| {
                    if let Some(p) = &c.location {
                        let _ = own.insert(p.as_str());
                    }
                });
                let mut n = 1;
                loop {
                    let candidate_stem = if n == 1 { stem.clone() } else { format!("{stem}-{n}") };
                    let candidate_dir = join_dir(&dir, &candidate_stem);
                    let prefix = format!("{candidate_dir}/");
                    let busy = taken
                        .iter()
                        .any(|t| t.starts_with(&prefix) && !own.contains(t.as_str()));
                    if !busy {
                        break ChapterPath::join(&candidate_dir, README)?;
                    }
                    n += 1;
                }
            } else {
                unique(&dir, &format!("{stem}.md"), taken)?
            };
            let _ = taken.insert(target.as_str().to_owned());
            let _ = assigned.insert(slot.clone(), target);
        }
        for index in 0..chapter.children.len() {
            self.plan(&slot.child(index), stems, taken, assigned)?;
        }
        Ok(())
    }

    /// Renames files (`old → new`) in the summary, manifest and file map,
    /// and rewrites relative links to and from the moved files.
    pub(crate) fn relocate(&mut self, moves: &BTreeMap<ChapterPath, ChapterPath>) {
        if moves.is_empty() {
            return;
        }
        let reverse: BTreeMap<&ChapterPath, &ChapterPath> =
            moves.iter().map(|(old, new)| (new, old)).collect();
        // The set of chapter paths before the move, to tell chapters from
        // assets when resolving links.
        let before: BTreeSet<String> = self.files.keys().map(|p| p.as_str().to_owned()).collect();
        let mut files = BTreeMap::new();
        for (path, text) in std::mem::take(&mut self.files) {
            let new = moves.get(&path).cloned().unwrap_or(path);
            let _ = files.insert(new, text);
        }
        let mut rewritten = BTreeMap::new();
        for (path, text) in &files {
            let old_dir = reverse.get(path).map_or_else(|| path.dir(), |old| old.dir()).to_owned();
            let new_dir = path.dir();
            let edits = rewrite_relative(text, &old_dir, new_dir, moves, &before);
            if !edits.is_empty() {
                let _ = rewritten.insert(path.clone(), markdown::apply(text, edits));
            }
        }
        files.extend(rewritten);
        self.files = files;
        for entry in self.manifest.nodes.values_mut() {
            if let Some(new) = moves.get(&entry.path) {
                entry.path = new.clone();
            }
        }
        rewrite_summary(&mut self.summary, moves);
        for old in moves.keys() {
            let _ = self.moved_away.insert(old.clone());
        }
        for new in moves.values() {
            let _ = self.moved_away.remove(new);
        }
    }
}

fn rewrite_summary(summary: &mut crate::summary::Summary, moves: &BTreeMap<ChapterPath, ChapterPath>) {
    fn visit(chapter: &mut Chapter, moves: &BTreeMap<ChapterPath, ChapterPath>) {
        if let Some(new) = chapter.location.as_ref().and_then(|p| moves.get(p)) {
            chapter.location = Some(new.clone());
        }
        for child in &mut chapter.children {
            visit(child, moves);
        }
    }
    for section in [Section::Prefix, Section::Numbered, Section::Suffix] {
        for item in summary.items_mut(section) {
            if let Item::Chapter(chapter) = item {
                visit(chapter, moves);
            }
        }
    }
}

/// Edits that keep relative links in one file pointing at the same
/// targets after files moved.
pub(crate) fn rewrite_relative(
    text: &str,
    old_dir: &str,
    new_dir: &str,
    moves: &BTreeMap<ChapterPath, ChapterPath>,
    chapters_before: &BTreeSet<String>,
) -> Vec<(std::ops::Range<usize>, String)> {
    let mut edits = Vec::new();
    for link in markdown::links(text) {
        if link.kind == LinkKind::Autolink {
            continue;
        }
        let (target, fragment) = match link.dest.split_once('#') {
            Some((target, fragment)) => (target, Some(fragment)),
            None => (link.dest.as_str(), None),
        };
        let Some(resolved) = path::resolve(old_dir, target) else {
            continue;
        };
        let moved_target = ChapterPath::parse(&resolved)
            .ok()
            .filter(|p| chapters_before.contains(p.as_str()))
            .and_then(|p| moves.get(&p).map(|n| n.as_str().to_owned()));
        let new_target = moved_target.unwrap_or(resolved);
        let mut href = path::relative(new_dir, &new_target);
        if let Some(fragment) = fragment {
            href.push('#');
            href.push_str(fragment);
        }
        if href != link.dest {
            edits.push((link.dest_range, href));
        }
    }
    edits
}

pub(crate) fn adjust_after_removal(dest: &mut Slot, removed: &Slot) {
    let level = removed.path.len() - 1;
    if dest.section != removed.section || dest.path.len() <= level {
        return;
    }
    if dest.path.get(..level) != removed.path.get(..level) {
        return;
    }
    if let (Some(d), Some(r)) = (dest.path.get_mut(level), removed.path.get(level))
        && *d > *r
    {
        *d -= 1;
    }
}

fn join_dir(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

pub(crate) fn unique(dir: &str, file: &str, taken: &BTreeSet<String>) -> Result<ChapterPath> {
    let stem = file.trim_end_matches(".md");
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem}-{n}.md")
        };
        let candidate = ChapterPath::join(dir, &name)?;
        if !taken.contains(candidate.as_str()) {
            return Ok(candidate);
        }
        n += 1;
    }
}

pub(crate) fn clean_title(title: &str) -> Result<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(BookError::NotAllowed("a title cannot be empty".to_owned()));
    }
    Ok(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};
    use crate::id::SequentialIds;

    fn paths(book: &Book) -> Vec<String> {
        book.files().keys().map(|p| p.as_str().to_owned()).collect()
    }

    fn assert_consistent(book: &Book) {
        for (id, entry) in &book.manifest().nodes {
            let text = book.files().get(&entry.path).expect("file for every node");
            assert_eq!(
                markdown::front_matter(text, "id").as_deref(),
                Some(id.as_str())
            );
            assert!(book.summary().find(&entry.path).is_some());
        }
        assert_eq!(book.files().len(), book.manifest().nodes.len());
        let again = crate::summary::Summary::parse(&book.summary_text()).unwrap();
        assert_eq!(&again, book.summary());
    }

    #[test]
    fn nest_a_leaf_turns_the_parent_into_a_folder() {
        let mut book = handbook();
        // Glossary (suffix) moves under Services (a leaf).
        book.move_section(&id(6).into(), &Place::LastChildOf(id(5).into()))
            .unwrap();
        assert_eq!(book.path_of(&id(5)).unwrap().as_str(), "architecture/services/README.md");
        assert_eq!(
            book.path_of(&id(6)).unwrap().as_str(),
            "architecture/services/glossary.md"
        );
        assert_eq!(book.label_of(&id(6)).unwrap(), "§2.1.1 Glossary");
        // The relative link from services to the glossary still resolves.
        let services = book.text(&id(5)).unwrap();
        assert!(services.contains("[the glossary](glossary.md)"), "{services}");
        assert!(book.moved_away().iter().any(|p| p.as_str() == "glossary.md"));
        assert_consistent(&book);
    }

    #[test]
    fn unnest_turns_the_folder_back_into_a_leaf() {
        let mut book = handbook();
        book.move_section(&id(5).into(), &Place::After(id(4).into()))
            .unwrap();
        assert_eq!(book.path_of(&id(4)).unwrap().as_str(), "architecture.md");
        assert_eq!(book.path_of(&id(5)).unwrap().as_str(), "services.md");
        assert_eq!(book.label_of(&id(5)).unwrap(), "§3 Services");
        assert!(book.text(&id(5)).unwrap().contains("(glossary.md)"));
        assert_consistent(&book);
    }

    #[test]
    fn reorder_siblings() {
        let mut book = handbook();
        book.move_section(&id(4).into(), &Place::Before(id(2).into()))
            .unwrap();
        assert_eq!(book.label_of(&id(4)).unwrap(), "§1 Architecture");
        assert_eq!(book.label_of(&id(3)).unwrap(), "§2.1 Your first week");
        // After(itself) is a no-op.
        book.move_section(&id(4).into(), &Place::After(id(4).into()))
            .unwrap();
        assert_eq!(book.label_of(&id(4)).unwrap(), "§1 Architecture");
        assert_consistent(&book);
    }

    #[test]
    fn refuses_bad_moves() {
        let mut book = handbook();
        assert!(book
            .move_section(&id(2).into(), &Place::LastChildOf(id(3).into()))
            .is_err());
        // Prefix chapters cannot have children.
        assert!(book
            .move_section(&id(6).into(), &Place::LastChildOf(id(1).into()))
            .is_err());
        // Nothing changed.
        assert_eq!(book, handbook());
        book.set_max_depth(2).unwrap();
        assert!(matches!(
            book.move_section(&id(2).into(), &Place::LastChildOf(id(5).into())),
            Err(BookError::TooDeep { .. } | BookError::NotAllowed(_))
        ));
        assert!(matches!(
            book.move_section(&id(6).into(), &Place::LastChildOf(id(5).into())),
            Err(BookError::TooDeep { max: 2, needed: 3 })
        ));
    }

    #[test]
    fn add_rename_and_remove() {
        let mut book = handbook();
        let mut ids = SequentialIds::new("01JBK00007");
        let new = book
            .add_section(&Place::LastChildOf(id(4).into()), "Data stores", "Postgres.", &mut ids)
            .unwrap();
        assert_eq!(book.path_of(&new).unwrap().as_str(), "architecture/data-stores.md");
        assert_eq!(book.label_of(&new).unwrap(), "§2.2 Data stores");
        book.rename(&id(4).into(), "System design").unwrap();
        assert_eq!(book.path_of(&id(4)).unwrap().as_str(), "system-design/README.md");
        assert_eq!(book.path_of(&new).unwrap().as_str(), "system-design/data-stores.md");
        assert_eq!(
            markdown::front_matter(book.text(&id(4)).unwrap(), "title").as_deref(),
            Some("System design")
        );
        assert!(book.text(&id(5)).unwrap().contains("(../glossary.md)"));
        let removed = book.remove_section(&id(4).into()).unwrap();
        assert_eq!(removed.len(), 3);
        assert!(book.removed().iter().any(|p| p.as_str() == "system-design/data-stores.md"));
        assert!(!paths(&book).iter().any(|p| p.starts_with("system-design")));
        assert_consistent(&book);
    }

    #[test]
    fn drafts_parts_and_separators() {
        let mut book = handbook();
        let mut ids = SequentialIds::new("01JBK00007");
        let draft_slot = book.resolve(&NodeRef::Slot(Slot {
            section: Section::Numbered,
            path: vec![1, 1],
        }));
        let draft_slot = draft_slot.unwrap();
        let new = book.write_draft(&draft_slot, "Phishing drills.", &mut ids).unwrap();
        assert_eq!(book.path_of(&new).unwrap().as_str(), "onboarding/security-training.md");
        let later = book
            .add_draft(&Place::End(Section::Numbered), "Roadmap")
            .unwrap();
        assert_eq!(book.summary().chapter(&later).unwrap().location, None);
        book.add_part(0, "Part 0 · Before").unwrap();
        book.add_separator(Section::Prefix, 1).unwrap();
        assert!(book.summary_text().contains("# Part 0 · Before"));
        book.remove_marker(&Slot::top(Section::Prefix, 1)).unwrap();
        assert!(book.remove_marker(&Slot::top(Section::Prefix, 0)).is_err());
        assert_consistent(&book);
    }

    #[test]
    fn slot_adjustment() {
        let mut dest = Slot { section: Section::Numbered, path: vec![3, 1] };
        adjust_after_removal(&mut dest, &Slot::top(Section::Numbered, 1));
        assert_eq!(dest.path, vec![2, 1]);
        let mut same_level = Slot { section: Section::Numbered, path: vec![1, 4] };
        adjust_after_removal(&mut same_level, &Slot { section: Section::Numbered, path: vec![1, 2] });
        assert_eq!(same_level.path, vec![1, 3]);
    }
}
