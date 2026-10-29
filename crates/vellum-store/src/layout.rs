// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive's on-disk layout ("what you see is the disk").
//!
//! ```text
//! .vellum/                      index.sqlite, trash/, keys/ (never content)
//! drives/shared/<drive>/        shared drives, each folder with .vellum-folder.yaml
//! drives/users/<user>/          My Drive
//! teams/<KEY>/issues/KEY-n.md   one file per issue
//! tasks/<user>/*.md             personal task lists
//! name.book/                    a book: book.yaml + SUMMARY.md + chapters
//! name.collection/              a collection: schema.yaml + rows/*.md
//! name.shortcut.yaml            an ID link that never grants access
//! ```
//!
//! Identity is the front-matter ULID, never the path, so this module only
//! names places and classifies what it finds there.

use crate::{error::StoreResult, path::BlobPath};

/// Directory holding Vellum's own state; never content.
pub const SYSTEM_DIR: &str = ".vellum";
/// Per-folder metadata file (id, icon, ACL copy).
pub const FOLDER_META: &str = ".vellum-folder.yaml";
/// A book's generated `mdBook` output; rebuilt on demand, never content.
pub const BUILD_DIR: &str = ".mdbook";
/// A book's published snapshots: data the books service owns, not
/// documents to index.
pub const SNAPSHOTS_DIR: &str = ".snapshots";
/// Longest file name, in bytes, that every supported backend accepts.
pub const MAX_NAME_BYTES: usize = 200;

/// What a path holds, decided from its location and name alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Vellum's own state under `.vellum/`, a VCS directory, or generated
    /// build output such as a book's `.mdbook/`.
    System,
    /// Editor scratch files (`.swp`, `~`, `.DS_Store`, ...); never ingested.
    Scratch,
    /// A folder's `.vellum-folder.yaml`.
    FolderMeta,
    /// A book's `book.yaml` or `SUMMARY.md`.
    BookManifest {
        /// The `name.book` directory.
        book: BlobPath,
    },
    /// A collection's `schema.yaml`.
    CollectionSchema {
        /// The `name.collection` directory.
        collection: BlobPath,
    },
    /// A shortcut (`name.shortcut.yaml`).
    Shortcut,
    /// An issue file, `teams/<KEY>/issues/<KEY>-<n>.md`.
    Issue {
        /// Team key, such as `ENG`.
        team: String,
        /// Issue number.
        number: u64,
    },
    /// A Vellum Markdown document (page, chapter, collection row, task list).
    Document,
    /// Anything else: images, attachments.
    Asset,
}

impl Kind {
    /// Classifies `path`.
    #[must_use]
    pub fn of(path: &BlobPath) -> Self {
        let name = path.file_name();
        let first = path.segments().next().unwrap_or_default();
        if first == SYSTEM_DIR || path.segments().any(|s| s == ".git" || s == BUILD_DIR) {
            return Self::System;
        }
        if is_scratch(name) {
            return Self::Scratch;
        }
        let parent = path.parent();
        let parent_name = parent.as_ref().map_or("", BlobPath::file_name);
        match name {
            FOLDER_META => return Self::FolderMeta,
            "book.yaml" | "SUMMARY.md" if has_ext(parent_name, "book") => {
                return parent.map_or(Self::Document, |book| Self::BookManifest { book });
            }
            "schema.yaml" if has_ext(parent_name, "collection") => {
                return parent.map_or(Self::Asset, |collection| Self::CollectionSchema {
                    collection,
                });
            }
            _ => {}
        }
        if in_book_snapshots(path) {
            return Self::Asset;
        }
        if name.to_ascii_lowercase().ends_with(".shortcut.yaml") {
            return Self::Shortcut;
        }
        if let Some(issue) = issue_of(path) {
            return issue;
        }
        if has_ext(name, "md") {
            Self::Document
        } else {
            Self::Asset
        }
    }

    /// True for paths the watcher and indexer should ingest.
    #[must_use]
    pub const fn is_content(&self) -> bool {
        !matches!(self, Self::System | Self::Scratch)
    }
}

fn has_ext(name: &str, ext: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, e)| !stem.is_empty() && e.eq_ignore_ascii_case(ext))
}

/// True for anything under `<name>.book/.snapshots/`.
fn in_book_snapshots(path: &BlobPath) -> bool {
    let segments: Vec<&str> = path.segments().collect();
    segments
        .windows(2)
        .any(|w| matches!(w, [book, snaps] if *snaps == SNAPSHOTS_DIR && has_ext(book, "book")))
}

fn is_scratch(name: &str) -> bool {
    const NAMES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];
    const EXTS: [&str; 5] = ["swp", "swx", "tmp", "crdownload", "part"];
    NAMES.contains(&name)
        || name.starts_with(".#")
        || name.starts_with("~$")
        || name.ends_with('~')
        || EXTS.iter().any(|e| has_ext(name, e))
}

fn issue_of(path: &BlobPath) -> Option<Kind> {
    let mut seg = path.segments();
    let (teams, team, issues, file) = (seg.next()?, seg.next()?, seg.next()?, seg.next()?);
    if teams != "teams" || issues != "issues" || seg.next().is_some() {
        return None;
    }
    let (key, number) = file.strip_suffix(".md")?.rsplit_once('-')?;
    let number = number.parse().ok()?;
    (key == team).then(|| Kind::Issue {
        team: team.to_owned(),
        number,
    })
}

/// Where things live.
pub mod at {
    use super::{BlobPath, FOLDER_META, SYSTEM_DIR, StoreResult};

    /// `.vellum/trash/`.
    ///
    /// # Errors
    /// Never in practice; the path is constant.
    pub fn trash() -> StoreResult<BlobPath> {
        BlobPath::new(format!("{SYSTEM_DIR}/trash"))
    }

    /// `drives/shared/<drive>/`.
    ///
    /// # Errors
    /// Returns `InvalidPath` if `drive` is not a valid name.
    pub fn shared_drive(drive: &str) -> StoreResult<BlobPath> {
        BlobPath::new(format!("drives/shared/{drive}"))
    }

    /// `drives/users/<user>/`, the user's My Drive.
    ///
    /// # Errors
    /// Returns `InvalidPath` if `user` is not a valid name.
    pub fn my_drive(user: &str) -> StoreResult<BlobPath> {
        BlobPath::new(format!("drives/users/{user}"))
    }

    /// `teams/<KEY>/issues/<KEY>-<n>.md`.
    ///
    /// # Errors
    /// Returns `InvalidPath` if `team` is not a valid key.
    pub fn issue(team: &str, number: u64) -> StoreResult<BlobPath> {
        BlobPath::new(format!("teams/{team}/issues/{team}-{number}.md"))
    }

    /// `tasks/<user>/`.
    ///
    /// # Errors
    /// Returns `InvalidPath` if `user` is not a valid name.
    pub fn tasks(user: &str) -> StoreResult<BlobPath> {
        BlobPath::new(format!("tasks/{user}"))
    }

    /// The `.vellum-folder.yaml` of `dir`.
    ///
    /// # Errors
    /// Returns `InvalidPath` if the result is too long.
    pub fn folder_meta(dir: &BlobPath) -> StoreResult<BlobPath> {
        dir.join(FOLDER_META)
    }
}

/// Turns a title into a file name every backend accepts.
///
/// The name stays readable: characters that Windows, macOS, S3 or Drive reject are
/// replaced with `-`, leading dots and trailing dots or spaces are
/// trimmed, Windows device names are suffixed, and the stem is cut on a
/// character boundary so the name with `ext` fits [`MAX_NAME_BYTES`].
#[must_use]
pub fn file_name(title: &str, ext: &str) -> String {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut stem = clean
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_owned();
    let upper = stem.to_ascii_uppercase();
    let device = RESERVED.contains(&upper.as_str())
        || (upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")));
    if stem.is_empty() || device {
        stem.push_str("-untitled");
        stem = stem.trim_start_matches('-').to_owned();
    }
    let budget = MAX_NAME_BYTES.saturating_sub(ext.len() + 1);
    while stem.len() > budget {
        let _ = stem.pop();
    }
    format!("{}.{ext}", stem.trim_end_matches(['.', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(p: &str) -> Kind {
        BlobPath::new(p).map_or(Kind::Asset, |p| Kind::of(&p))
    }

    #[test]
    fn classifies_the_layout() -> StoreResult<()> {
        assert_eq!(kind(".vellum/index.sqlite"), Kind::System);
        // `.git` cannot even be named as a blob path (#272).
        assert!(BlobPath::new("drives/shared/Ops/.git/HEAD").is_err());
        assert_eq!(kind("drives/users/ana/.Plan.md.swp"), Kind::Scratch);
        assert_eq!(kind("drives/users/ana/Plan.md~"), Kind::Scratch);
        assert_eq!(
            kind("drives/shared/Ops/.vellum-folder.yaml"),
            Kind::FolderMeta
        );
        let book = BlobPath::new("drives/shared/Ops/Guide.book")?;
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/SUMMARY.md"),
            Kind::BookManifest { book }
        );
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/intro.md"),
            Kind::Document
        );
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/.mdbook/index.html"),
            Kind::System
        );
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/.snapshots/v1/intro.md"),
            Kind::Asset
        );
        assert_eq!(
            kind("drives/shared/Ops/Notes/.snapshots/intro.md"),
            Kind::Document
        );
        let collection = BlobPath::new("drives/shared/Ops/Vendors.collection")?;
        let schema = "drives/shared/Ops/Vendors.collection/schema.yaml";
        assert_eq!(kind(schema), Kind::CollectionSchema { collection });
        assert_eq!(
            kind("drives/users/ana/Roadmap.shortcut.yaml"),
            Kind::Shortcut
        );
        let issue = Kind::Issue {
            team: "ENG".into(),
            number: 142,
        };
        assert_eq!(kind("teams/ENG/issues/ENG-142.md"), issue);
        assert_eq!(kind("teams/ENG/issues/OPS-1.md"), Kind::Document);
        assert_eq!(kind("tasks/ana/Errands.md"), Kind::Document);
        assert_eq!(kind("drives/users/ana/logo.png"), Kind::Asset);
        assert!(!Kind::System.is_content() && Kind::Asset.is_content());
        Ok(())
    }

    #[test]
    fn builds_paths() -> StoreResult<()> {
        assert_eq!(
            at::issue("ENG", 142)?.as_str(),
            "teams/ENG/issues/ENG-142.md"
        );
        assert_eq!(at::my_drive("ana")?.as_str(), "drives/users/ana");
        assert_eq!(
            at::folder_meta(&at::shared_drive("Ops")?)?.as_str(),
            "drives/shared/Ops/.vellum-folder.yaml"
        );
        assert_eq!(at::tasks("ana")?.as_str(), "tasks/ana");
        assert_eq!(at::trash()?.as_str(), ".vellum/trash");
        Ok(())
    }

    #[test]
    fn file_names_are_portable() {
        assert_eq!(file_name("Q3 plan: draft?", "md"), "Q3 plan- draft-.md");
        assert_eq!(file_name("  ..hidden. ", "md"), "hidden.md");
        assert_eq!(file_name("", "md"), "untitled.md");
        assert_eq!(file_name("con", "md"), "con-untitled.md");
        let long = file_name(&"é".repeat(300), "md");
        assert!(
            long.len() <= MAX_NAME_BYTES && long.ends_with("é.md"),
            "{long}"
        );
    }
}
