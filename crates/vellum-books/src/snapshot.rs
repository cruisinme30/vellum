// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Book snapshots and tags, and the diff between two of them (R-BK-8).
//!
//! A snapshot freezes the whole set (manifest, summary and every chapter)
//! under an immutable tag such as `v1.0`. It is one JSON file in the
//! book folder at `.snapshots/<tag>.json`, so it travels with the book
//! (Git, backups) and never depends on the version store's retention.
//! Per-leaf and per-manifest history stays with the version service
//! (R-BK-2); snapshots are the named, whole-book points on top of it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use similar::TextDiff;

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::{BookId, NodeId, SequentialIds};
use crate::path::ChapterPath;

/// The folder inside a book that holds snapshots.
pub const DIR: &str = ".snapshots";

/// Checks a tag: 1 to 64 characters, letters, digits, `.`, `_` and `-`,
/// starting with a letter or digit.
///
/// # Errors
///
/// Returns [`BookError::Tag`] when the tag is malformed.
pub fn validate_tag(tag: &str) -> Result<()> {
    let ok = !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(BookError::Tag {
            tag: tag.to_owned(),
            reason: "use 1 to 64 letters, digits, '.', '_' or '-', starting \
                with a letter or digit",
        })
    }
}

/// The relative path of a tag's snapshot file inside the book folder.
///
/// # Errors
///
/// Returns [`BookError::Tag`] when the tag is malformed.
pub fn file_name(tag: &str) -> Result<String> {
    validate_tag(tag)?;
    Ok(format!("{DIR}/{tag}.json"))
}

/// A frozen copy of a whole book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// The format version; always 1.
    pub vellum: u32,
    /// The book.
    pub book_id: BookId,
    /// The tag, such as `v1.0`.
    pub tag: String,
    /// What this snapshot is for.
    pub message: String,
    /// When it was taken (RFC 3339).
    pub created_at: String,
    /// Who took it (`user:<name>`).
    pub created_by: String,
    /// The text of `book.yaml`.
    pub manifest: String,
    /// The text of `SUMMARY.md`.
    pub summary: String,
    /// Every chapter file by path.
    pub files: BTreeMap<String, String>,
}

impl Snapshot {
    /// Serialises the snapshot as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Snapshot`] if serialisation fails.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| BookError::Snapshot(e.to_string()))
    }

    /// Parses a snapshot file.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Snapshot`] for malformed JSON or an unknown
    /// version, and [`BookError::Tag`] for a malformed tag.
    pub fn from_json(text: &str) -> Result<Self> {
        let snapshot: Self = serde_json::from_str(text)
            .map_err(|e| BookError::Snapshot(e.to_string()))?;
        if snapshot.vellum != 1 {
            return Err(BookError::Snapshot(format!(
                "unsupported version {}",
                snapshot.vellum
            )));
        }
        validate_tag(&snapshot.tag)?;
        Ok(snapshot)
    }

    /// The book as it was when the snapshot was taken.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot's files are malformed.
    pub fn book(&self) -> Result<Book> {
        // A snapshot is always consistent, so no ids are minted; the
        // source is only a formality.
        let mut ids = SequentialIds::default();
        let (book, _) = Book::load(
            &self.manifest,
            &self.summary,
            |path: &ChapterPath| self.files.get(path.as_str()).cloned(),
            &mut ids,
        )?;
        Ok(book)
    }
}

impl Book {
    /// Takes a snapshot of the whole book under `tag`.
    ///
    /// `existing` lists the tags already taken; tags are immutable, so
    /// reusing one is refused.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Tag`] for a malformed or reused tag.
    pub fn snapshot(
        &self,
        tag: &str,
        message: &str,
        created_at: &str,
        created_by: &str,
        existing: &BTreeSet<String>,
    ) -> Result<Snapshot> {
        validate_tag(tag)?;
        if existing.contains(tag) {
            return Err(BookError::Tag {
                tag: tag.to_owned(),
                reason: "already exists; tags are immutable",
            });
        }
        Ok(Snapshot {
            vellum: 1,
            book_id: self.manifest.id.clone(),
            tag: tag.to_owned(),
            message: message.to_owned(),
            created_at: created_at.to_owned(),
            created_by: created_by.to_owned(),
            manifest: self.manifest_text()?,
            summary: self.summary_text(),
            files: self
                .files
                .iter()
                .map(|(path, text)| (path.as_str().to_owned(), text.clone()))
                .collect(),
        })
    }
}

/// A section that exists on one side of a diff only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionRef {
    /// The section.
    pub id: NodeId,
    /// Its label on that side, such as `§2.3 Services`.
    pub label: String,
}

/// A section whose place in the tree changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Moved {
    /// The section.
    pub id: NodeId,
    /// Its label before.
    pub from: String,
    /// Its label after.
    pub to: String,
}

/// A section whose text changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Changed {
    /// The section.
    pub id: NodeId,
    /// Its label after the change.
    pub label: String,
    /// Lines added.
    pub added: usize,
    /// Lines removed.
    pub removed: usize,
    /// A unified diff of the file.
    pub patch: String,
}

/// What changed between two versions of a book.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct BookDiff {
    /// Sections only in the newer version.
    pub added: Vec<SectionRef>,
    /// Sections only in the older version.
    pub removed: Vec<SectionRef>,
    /// Sections that moved, were renumbered or renamed.
    pub moved: Vec<Moved>,
    /// Sections whose text changed.
    pub changed: Vec<Changed>,
    /// Book settings that changed (`title`, `numbering`, `max_depth`,
    /// `export_profile`, section access).
    pub settings: Vec<String>,
    /// A unified diff of `SUMMARY.md`, empty when unchanged.
    pub summary_patch: String,
}

impl BookDiff {
    /// Whether nothing changed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.moved.is_empty()
            && self.changed.is_empty()
            && self.settings.is_empty()
            && self.summary_patch.is_empty()
    }
}

fn unified(
    old: &str,
    new: &str,
    old_name: &str,
    new_name: &str,
) -> (String, usize, usize) {
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0;
    let mut removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    if added == 0 && removed == 0 {
        return (String::new(), 0, 0);
    }
    let patch = diff
        .unified_diff()
        .context_radius(3)
        .header(old_name, new_name)
        .to_string();
    (patch, added, removed)
}

/// Compares two versions of a book (two snapshots, or a snapshot and the
/// live book). Sections are matched by id, so a renamed or moved file is
/// a move, not a removal and an addition.
#[must_use]
pub fn diff(old: &Book, new: &Book) -> BookDiff {
    let mut out = BookDiff::default();
    let label = |book: &Book, id: &NodeId| {
        book.label_of(id).unwrap_or_else(|_| id.to_string())
    };
    for id in new.manifest.nodes.keys() {
        if !old.contains(id) {
            out.added.push(SectionRef {
                id: id.clone(),
                label: label(new, id),
            });
        }
    }
    for id in old.manifest.nodes.keys() {
        if !new.contains(id) {
            out.removed.push(SectionRef {
                id: id.clone(),
                label: label(old, id),
            });
        }
    }
    for (id, old_entry) in &old.manifest.nodes {
        let Some(new_entry) = new.manifest.nodes.get(id) else {
            continue;
        };
        let (before, after) = (label(old, id), label(new, id));
        let old_parent = old
            .slot_of(id)
            .ok()
            .and_then(|s| s.parent())
            .and_then(|p| old.id_of_slot(&p).cloned());
        let new_parent = new
            .slot_of(id)
            .ok()
            .and_then(|s| s.parent())
            .and_then(|p| new.id_of_slot(&p).cloned());
        if before != after || old_parent != new_parent {
            out.moved.push(Moved {
                id: id.clone(),
                from: before,
                to: after.clone(),
            });
        }
        let old_text = old.text(id).unwrap_or_default();
        let new_text = new.text(id).unwrap_or_default();
        let (patch, added, removed) = unified(
            old_text,
            new_text,
            &format!("a/{}", old_entry.path),
            &format!("b/{}", new_entry.path),
        );
        if !patch.is_empty() {
            out.changed.push(Changed {
                id: id.clone(),
                label: after,
                added,
                removed,
                patch,
            });
        }
        if (
            &old_entry.owners,
            &old_entry.reviewers,
            &old_entry.restrict_to,
        ) != (
            &new_entry.owners,
            &new_entry.reviewers,
            &new_entry.restrict_to,
        ) {
            out.settings.push(format!("access of {}", label(new, id)));
        }
    }
    let (o, n) = (&old.manifest, &new.manifest);
    if o.title != n.title {
        out.settings.push("title".to_owned());
    }
    if o.numbering != n.numbering {
        out.settings.push("numbering".to_owned());
    }
    if o.max_depth != n.max_depth {
        out.settings.push("max_depth".to_owned());
    }
    if o.export_profile != n.export_profile {
        out.settings.push("export_profile".to_owned());
    }
    out.summary_patch = unified(
        &old.summary_text(),
        &new.summary_text(),
        "a/SUMMARY.md",
        "b/SUMMARY.md",
    )
    .0;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};
    use crate::id::SequentialIds;
    use crate::ops::Place;

    #[test]
    fn snapshot_round_trip() {
        let book = handbook();
        let snap = book
            .snapshot(
                "v1.0",
                "First release",
                "2026-10-29T08:00:00Z",
                "user:maya",
                &BTreeSet::new(),
            )
            .unwrap();
        let json = snap.to_json().unwrap();
        let again = Snapshot::from_json(&json).unwrap();
        assert_eq!(again, snap);
        assert_eq!(again.book().unwrap(), book);
        assert_eq!(file_name("v1.0").unwrap(), ".snapshots/v1.0.json");
    }

    #[test]
    fn tags_are_checked_and_immutable() {
        let book = handbook();
        for bad in ["", "../x", ".hidden", "a b", &"x".repeat(65)] {
            assert!(validate_tag(bad).is_err(), "{bad}");
        }
        let existing = BTreeSet::from(["v1.0".to_owned()]);
        assert!(
            book.snapshot("v1.0", "", "", "user:maya", &existing)
                .is_err()
        );
    }

    #[test]
    fn diff_two_tags() {
        let v1 = handbook();
        let mut v2 = v1.clone();
        let mut ids = SequentialIds::new("01JBK00003");
        v2.move_section(&id(4).into(), &Place::Before(id(2).into()))
            .unwrap();
        v2.rename(&id(6).into(), "Terms").unwrap();
        let added = v2
            .add_section(
                &Place::End(crate::summary::Section::Numbered),
                "FAQ",
                "Q and A.",
                &mut ids,
            )
            .unwrap();
        let _ = v2.remove_section(&id(1).into()).unwrap();
        let text = format!("{}\nMore.\n", v2.text(&id(5)).unwrap());
        v2.set_text(&id(5), text).unwrap();
        v2.set_numbering(crate::manifest::NumberingStyle::Outline);

        let d = diff(&v1, &v2);
        assert_eq!(d.added.iter().map(|s| &s.id).collect::<Vec<_>>(), [&added]);
        assert_eq!(
            d.removed.iter().map(|s| &s.id).collect::<Vec<_>>(),
            [&id(1)]
        );
        let moved: Vec<(&str, &str)> = d
            .moved
            .iter()
            .map(|m| (m.from.as_str(), m.to.as_str()))
            .collect();
        assert!(
            moved.contains(&("§2 Architecture", "§I Architecture")),
            "{moved:?}"
        );
        assert!(moved.contains(&("Glossary", "Terms")), "{moved:?}");
        let services = d.changed.iter().find(|c| c.id == id(5)).unwrap();
        // Two lines added, and the link to the renamed glossary rewritten.
        assert_eq!((services.added, services.removed), (3, 1));
        assert!(services.patch.contains("+More."));
        assert!(
            services.patch.contains("+See [the glossary](../terms.md)"),
            "{}",
            services.patch
        );
        assert!(d.settings.contains(&"numbering".to_owned()));
        assert!(d.summary_patch.contains("-[Introduction](README.md)"));
        assert!(diff(&v1, &v1).is_empty());
    }
}
