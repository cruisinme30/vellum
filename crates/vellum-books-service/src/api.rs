// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Inputs and outputs of the books operations. Inputs are path parameters
//! plus the query string (GET, DELETE) or JSON body; every type documents
//! itself in the OpenAPI and GraphQL schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_books::toc::{TocEntry, TocItem};
use vellum_kernel::BookId;

/// `books.list` takes no input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListBooks {}

/// A book in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BookSummary {
    /// The book.
    pub id: BookId,
    /// Its title.
    pub title: String,
    /// The `.book` folder.
    pub path: String,
}

/// The books the caller can open, by title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BookList {
    /// The books.
    pub books: Vec<BookSummary>,
}

/// Names a book.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BookKey {
    /// The book.
    pub book_id: BookId,
}

/// One chapter of the table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TocNode {
    /// The section ID; absent for a draft.
    pub id: Option<String>,
    /// The navigation title.
    pub title: String,
    /// The cross-reference label, such as `§2.3 Services`.
    pub label: String,
    /// The number in the book's style (`2.3`, `II.C`), if numbered.
    pub number: Option<String>,
    /// Nesting depth, 1 at the top.
    pub depth: usize,
    /// The chapter file inside the book folder; absent for a draft.
    pub path: Option<String>,
    /// The part this chapter falls under.
    pub part: Option<String>,
    /// Whether the chapter is a draft without a file.
    pub draft: bool,
    /// Nested chapters.
    pub children: Vec<Self>,
}

impl From<&TocEntry> for TocNode {
    fn from(e: &TocEntry) -> Self {
        Self {
            id: e.id.as_ref().map(ToString::to_string),
            title: e.title.clone(),
            label: e.label.clone(),
            number: e.number_label.clone(),
            depth: e.depth,
            path: e.path.as_ref().map(|p| p.as_str().to_owned()),
            part: e.part.clone(),
            draft: e.draft,
            children: e.children.iter().map(Self::from).collect(),
        }
    }
}

/// A line of the table of contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TocLine {
    /// A chapter and its children.
    Chapter(TocNode),
    /// A part title.
    Part {
        /// The title.
        title: String,
    },
    /// A separator.
    Separator,
}

impl From<&TocItem> for TocLine {
    fn from(item: &TocItem) -> Self {
        match item {
            TocItem::Chapter(e) => Self::Chapter(TocNode::from(e.as_ref())),
            TocItem::Part { title } => Self::Part {
                title: title.clone(),
            },
            TocItem::Separator => Self::Separator,
        }
    }
}

/// A book as the caller sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct BookInfo {
    /// The book.
    pub id: BookId,
    /// Its title.
    pub title: String,
    /// `decimal`, `outline` or `none`.
    pub numbering: String,
    /// The deepest nesting allowed.
    pub max_depth: u8,
    /// The caller's role on the book: `viewer` to `owner`.
    pub role: String,
    /// The table of contents, without the sections the caller cannot see.
    pub toc: Vec<TocLine>,
}

/// Names a section.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SectionKey {
    /// The book.
    pub book_id: BookId,
    /// The section (its front-matter ID).
    pub section_id: String,
}

/// Who may see, own and review a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SectionAccessInfo {
    /// Who may see it (`user:<id>`, `group:<id>`), when it is restricted:
    /// the intersection of its own and every enclosing restriction.
    pub restrict_to: Option<Vec<String>>,
    /// The sections whose restrictions apply, outermost first.
    pub restricted_by: Vec<String>,
    /// Its own `restrict_to`, as `book.yaml` stores it.
    pub own_restrict_to: Option<Vec<String>>,
    /// Effective owners.
    pub owners: Vec<String>,
    /// Effective reviewers.
    pub reviewers: Vec<String>,
}

/// One section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SectionDoc {
    /// The book.
    pub book_id: BookId,
    /// The section.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its cross-reference label.
    pub label: String,
    /// Its number in the book's style, if numbered.
    pub number: Option<String>,
    /// Its file inside the book folder.
    pub path: String,
    /// The body with cross-references filled in (`§2.3 Services`).
    pub markdown: String,
    /// The whole file, front matter included.
    pub source: String,
    /// Whether a restriction applies to it.
    pub restricted: bool,
    /// Its access settings; only for book managers and owners.
    pub access: Option<SectionAccessInfo>,
}

/// Where to put a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum At {
    /// Just before `section`, as its sibling.
    Before,
    /// Just after `section`, as its sibling.
    After,
    /// As the first child of `section`.
    FirstChildOf,
    /// As the last child of `section`.
    LastChildOf,
    /// At the start of `region`.
    Start,
    /// At the end of `region`.
    End,
}

/// A region of the summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    /// Unnumbered chapters before the first numbered one.
    Prefix,
    /// The numbered chapters.
    Numbered,
    /// Unnumbered chapters after the numbered ones.
    Suffix,
}

/// A place in the tree: relative to a section, or at either end of a
/// region.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PlaceInput {
    /// Where.
    pub at: At,
    /// The section `before`, `after`, `first_child_of` and
    /// `last_child_of` refer to.
    pub section: Option<String>,
    /// The region `start` and `end` refer to; `numbered` by default.
    pub region: Option<Region>,
}

/// `books.add_section`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddSection {
    /// The book.
    pub book_id: BookId,
    /// The new section's title.
    pub title: String,
    /// Its Markdown body.
    #[serde(default)]
    pub body: String,
    /// Where it goes.
    pub place: PlaceInput,
}

/// `books.move_section`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MoveSection {
    /// The book.
    pub book_id: BookId,
    /// The section to move, with everything under it.
    pub section_id: String,
    /// Where it goes.
    pub place: PlaceInput,
}

/// `books.update_section`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateSection {
    /// The book.
    pub book_id: BookId,
    /// The section.
    pub section_id: String,
    /// A new title (renames the file too).
    pub title: Option<String>,
    /// A new Markdown body; the front matter is kept.
    pub body: Option<String>,
}

/// `books.set_section_access`: replaces the section's own settings.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetSectionAccess {
    /// The book.
    pub book_id: BookId,
    /// The section.
    pub section_id: String,
    /// Who may see it and everything under it (`user:<id>`,
    /// `group:<id>`); empty lifts its own restriction. It can only narrow
    /// an enclosing restriction.
    pub restrict_to: Vec<String>,
    /// Its owners; empty inherits them.
    #[serde(default)]
    pub owners: Vec<String>,
    /// Its reviewers; empty inherits them.
    #[serde(default)]
    pub reviewers: Vec<String>,
}

/// Sections a removal took out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Removed {
    /// The removed sections, the named one first.
    pub removed: Vec<String>,
}

/// A snapshot's header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SnapshotInfo {
    /// The tag, such as `v1.0`.
    pub tag: String,
    /// What it marks.
    pub message: String,
    /// When it was taken (RFC 3339).
    pub created_at: String,
    /// Who took it (`user:<id>`).
    pub created_by: String,
}

/// A book's snapshots, by tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SnapshotList {
    /// The snapshots.
    pub snapshots: Vec<SnapshotInfo>,
}

/// `books.create_snapshot`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateSnapshot {
    /// The book.
    pub book_id: BookId,
    /// The tag: letters, digits, `.`, `-` and `_`.
    pub tag: String,
    /// What it marks.
    #[serde(default)]
    pub message: String,
}

/// `books.diff`: what changed between a snapshot and another snapshot or
/// the current book.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DiffInput {
    /// The book.
    pub book_id: BookId,
    /// The older snapshot's tag.
    pub from: String,
    /// The newer snapshot's tag; the current book when absent.
    pub to: Option<String>,
}

/// A section on one side of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffSection {
    /// The section.
    pub id: String,
    /// Its label on that side.
    pub label: String,
}

/// A section that moved, was renumbered or renamed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffMoved {
    /// The section.
    pub id: String,
    /// Its label before.
    pub from: String,
    /// Its label after.
    pub to: String,
}

/// A section whose text changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffChanged {
    /// The section.
    pub id: String,
    /// Its label after the change.
    pub label: String,
    /// Lines added.
    pub added: usize,
    /// Lines removed.
    pub removed: usize,
    /// A unified diff of the file.
    pub patch: String,
}

/// What changed. Sections the caller cannot see are left out, and so are
/// the settings and summary changes unless the caller manages the book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DiffOutput {
    /// Sections only in the newer version.
    pub added: Vec<DiffSection>,
    /// Sections only in the older version.
    pub removed: Vec<DiffSection>,
    /// Sections that moved, were renumbered or renamed.
    pub moved: Vec<DiffMoved>,
    /// Sections whose text changed.
    pub changed: Vec<DiffChanged>,
    /// Book settings that changed.
    pub settings: Vec<String>,
    /// A unified diff of `SUMMARY.md`.
    pub summary_patch: String,
}

/// `books.history`: the book's outline changes, newest first.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HistoryInput {
    /// The book.
    pub book_id: BookId,
    /// The most entries to return, 1 to 200; 50 when absent.
    pub limit: Option<u16>,
}

/// One section in a recorded outline, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OutlineLine {
    /// The section.
    pub id: String,
    /// Its title then.
    pub title: String,
    /// Its cross-reference label then, such as `§2.3 Services`.
    pub label: String,
    /// Nesting depth, 1 at the top.
    pub depth: usize,
    /// The section it sat under, if any.
    pub parent: Option<String>,
    /// `prefix`, `numbered` or `suffix`.
    pub region: String,
}

/// A change to the book's manifest or summary: a reorder, a nest, an
/// added, renamed or removed section, or new section access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HistoryEntry {
    /// The entry, for `books.restore_outline`.
    pub id: String,
    /// When (RFC 3339).
    pub at: String,
    /// Who (`user:<id>`), empty for the system.
    pub by: String,
    /// The operation, such as `move_section`.
    pub op: String,
    /// The outline after the change, without hidden sections.
    pub outline: Vec<OutlineLine>,
}

/// The book's outline history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HistoryList {
    /// Entries, newest first.
    pub entries: Vec<HistoryEntry>,
}

/// `books.restore_outline`: puts every section that still exists back
/// where an earlier outline had it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RestoreOutline {
    /// The book.
    pub book_id: BookId,
    /// The history entry to restore.
    pub entry: String,
}

/// `books.transfer_section`: moves or copies a section and everything
/// under it into another book.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TransferSection {
    /// The source book.
    pub book_id: BookId,
    /// The section.
    pub section_id: String,
    /// The target book.
    pub to_book: BookId,
    /// Where it goes in the target book.
    pub place: PlaceInput,
    /// Copy with fresh IDs instead of moving.
    #[serde(default)]
    pub copy: bool,
}

/// `books.copy_section`: copies a section and everything under it within
/// its book, with fresh IDs.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CopySection {
    /// The book.
    pub book_id: BookId,
    /// The section.
    pub section_id: String,
    /// Where the copy goes.
    pub place: PlaceInput,
}

/// `books.split_section`: turns a section's headings into child sections.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SplitSection {
    /// The book.
    pub book_id: BookId,
    /// The section.
    pub section_id: String,
    /// The heading level to split at, 1 to 6; the shallowest present when
    /// absent.
    pub level: Option<u8>,
}

/// Sections a split, copy or transfer created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Created {
    /// The book they are in.
    pub book_id: BookId,
    /// The section that now holds the named content (the copy's root for
    /// a copy, the section itself for a move or split).
    pub section_id: String,
    /// New section IDs, in reading order.
    pub created: Vec<String>,
}

/// `books.merge_section`: folds a section's children back into it as
/// headings.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MergeSection {
    /// The book.
    pub book_id: BookId,
    /// The section whose children are folded in.
    pub section_id: String,
}

/// What a publication is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublishFormat {
    /// A static HTML site with no scripts.
    Site,
    /// An mdBook source tree for `mdbook build` or the public docs site.
    Mdbook,
}

/// `books.publish`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Publish {
    /// The book.
    pub book_id: BookId,
    /// What to build.
    pub format: PublishFormat,
    /// Who it is for (`user:<id>`, `group:<id>`): restricted sections are
    /// kept only for these. Empty publishes for everyone, and only a
    /// public site is served at `/published/books/<book id>/`.
    #[serde(default)]
    pub audience: Vec<String>,
}

/// A publication of the book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Publication {
    /// What was built.
    pub format: PublishFormat,
    /// Whether it is for everyone.
    pub public: bool,
    /// When (RFC 3339).
    pub at: String,
    /// Who (`user:<id>`).
    pub by: String,
    /// Where the files are in the store.
    pub folder: String,
    /// How many files it holds.
    pub files: usize,
    /// Where a public site is served, relative to the app origin.
    pub url: Option<String>,
}

/// The book's current publications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PublicationList {
    /// One per format, if published.
    pub publications: Vec<Publication>,
}
