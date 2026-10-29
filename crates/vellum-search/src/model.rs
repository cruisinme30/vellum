// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What goes into the index and what comes out of a search.
//!
//! The retrieval unit is a block; the context unit is the heading section
//! the block sits under (D62). Callers hand the index plain data
//! ([`IndexDocument`]), produced from the format AST by the pages service,
//! so this crate never parses Markdown itself.

use std::fmt;
use std::ops::RangeInclusive;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{BlockId, DocId, ObjectId, SpaceId, Timestamp, UserId};

/// The kind of object a document is, for the drive's "type" filter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// A page.
    Page,
    /// A book, or a page inside one.
    Book,
    /// A collection's own page.
    Collection,
    /// A collection row, indexed with its computed values.
    Row,
    /// An issue.
    Issue,
    /// A My Work checklist file.
    TaskList,
    /// A folder (its name is searchable, it has no blocks).
    Folder,
}

impl DocKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 7] = [
        Self::Page,
        Self::Book,
        Self::Collection,
        Self::Row,
        Self::Issue,
        Self::TaskList,
        Self::Folder,
    ];

    /// The stable lowercase name stored in every index.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Book => "book",
            Self::Collection => "collection",
            Self::Row => "row",
            Self::Issue => "issue",
            Self::TaskList => "task_list",
            Self::Folder => "folder",
        }
    }
}

impl fmt::Display for DocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DocKind {
    type Err = UnknownKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| UnknownKind(text.to_owned()))
    }
}

/// The kind of a block, which shapes how its hit is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    /// A heading; it opens a section.
    Heading,
    /// A paragraph.
    Paragraph,
    /// A list item.
    ListItem,
    /// A checklist item.
    Task,
    /// A quotation.
    Quote,
    /// A code block.
    Code,
    /// A table, flattened to text row by row.
    Table,
    /// A callout or other container block.
    Callout,
    /// An image or diagram caption.
    Caption,
    /// A property or computed value: `Field: value` for rows and issues.
    Property,
    /// Anything else with text.
    Other,
}

impl BlockKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 11] = [
        Self::Heading,
        Self::Paragraph,
        Self::ListItem,
        Self::Task,
        Self::Quote,
        Self::Code,
        Self::Table,
        Self::Callout,
        Self::Caption,
        Self::Property,
        Self::Other,
    ];

    /// The stable lowercase name stored in every index.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heading => "heading",
            Self::Paragraph => "paragraph",
            Self::ListItem => "list_item",
            Self::Task => "task",
            Self::Quote => "quote",
            Self::Code => "code",
            Self::Table => "table",
            Self::Callout => "callout",
            Self::Caption => "caption",
            Self::Property => "property",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for BlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BlockKind {
    type Err = UnknownKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| UnknownKind(text.to_owned()))
    }
}

/// A stored kind name that this build does not know.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown kind `{0}`")]
pub struct UnknownKind(pub String);

/// One block of a document, ready to index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexBlock {
    /// The block's stable ID.
    pub id: BlockId,
    /// What kind of block it is.
    pub kind: BlockKind,
    /// The heading that opens the section this block is in; `None` before
    /// the first heading. A heading's own section is itself.
    pub section: Option<BlockId>,
    /// Heading texts from the top of the document down to this block's section.
    pub heading_path: Vec<String>,
    /// Plain text of the block (formatting stripped, computed values rendered).
    pub text: String,
}

/// A document as the index sees it: metadata plus blocks in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDocument {
    /// The space the document lives in.
    pub space: SpaceId,
    /// The document.
    pub doc: DocId,
    /// Its kind.
    pub kind: DocKind,
    /// Its title.
    pub title: String,
    /// Its path inside the space, for display.
    pub path: String,
    /// Its owner, if it has one.
    pub owner: Option<UserId>,
    /// When its content last changed.
    pub modified: Timestamp,
    /// Every container above it: the space root, drives, folders and books,
    /// outermost first. The location filter matches any of them.
    pub ancestors: Vec<ObjectId>,
    /// Whether it is in the trash.
    pub trashed: bool,
    /// Its blocks, in reading order.
    pub blocks: Vec<IndexBlock>,
}

/// Drive search filters (R-DR-9). Every set filter must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filters {
    /// Only these spaces (intersected with what the person can read).
    pub spaces: Vec<SpaceId>,
    /// Only documents owned by one of these people.
    pub owners: Vec<UserId>,
    /// Only documents last modified in this range, in Unix milliseconds.
    pub modified: Option<RangeInclusive<i64>>,
    /// Only these kinds.
    pub kinds: Vec<DocKind>,
    /// Only documents under this folder, drive, book or space.
    pub location: Option<ObjectId>,
    /// Only documents the person has starred.
    pub starred: bool,
    /// Search the trash instead of live documents.
    pub trashed: bool,
}

/// A full-text search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// What the person typed.
    pub text: String,
    /// Filters to apply.
    pub filters: Filters,
    /// How many hits to skip.
    pub offset: usize,
    /// How many hits to return (clamped to [`MAX_LIMIT`]).
    pub limit: usize,
}

/// The most hits one request can return.
pub const MAX_LIMIT: usize = 100;

impl SearchQuery {
    /// A query for `text` with no filters, returning the first `limit` hits.
    #[must_use]
    pub fn new(text: impl Into<String>, limit: usize) -> Self {
        Self {
            text: text.into(),
            filters: Filters::default(),
            offset: 0,
            limit,
        }
    }
}

/// A run of snippet text, highlighted when it matched the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Fragment {
    /// The text.
    pub text: String,
    /// Whether it matched a query term.
    pub hit: bool,
}

/// One block that matched, with what the UI and MCP need to show and cite it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchHit {
    /// The space.
    pub space: SpaceId,
    /// The document.
    pub doc: DocId,
    /// The block.
    pub block: BlockId,
    /// The heading that opens the block's section.
    pub section: Option<BlockId>,
    /// The document's kind.
    pub doc_kind: DocKind,
    /// The block's kind.
    pub block_kind: BlockKind,
    /// The document's title.
    pub title: String,
    /// The document's path.
    pub path: String,
    /// Headings above the block.
    pub heading_path: Vec<String>,
    /// A short excerpt of the block with the matches marked.
    pub snippet: Vec<Fragment>,
    /// The fused relevance score; higher is better. Only comparable within one response.
    pub score: f64,
    /// A stable citation: `vellum://doc/{doc}#{block}`.
    pub citation: String,
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchPage {
    /// The hits, best first.
    pub hits: Vec<SearchHit>,
    /// Whether more hits follow this page.
    pub has_more: bool,
    /// Whether semantic ranking took part.
    pub hybrid: bool,
}

/// A title match for the instant (⌘K) search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TitleHit {
    /// The space.
    pub space: SpaceId,
    /// The document.
    pub doc: DocId,
    /// Its kind.
    pub kind: DocKind,
    /// Its title.
    pub title: String,
    /// Its path.
    pub path: String,
}

/// A block as stored in an index: what a backend returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredBlock {
    /// The space.
    pub space: SpaceId,
    /// The document.
    pub doc: DocId,
    /// The block.
    pub block: BlockId,
    /// Position of the block in the document.
    pub position: u32,
    /// The section heading.
    pub section: Option<BlockId>,
    /// The document's kind.
    pub doc_kind: DocKind,
    /// The block's kind.
    pub block_kind: BlockKind,
    /// The document's title.
    pub title: String,
    /// The document's path.
    pub path: String,
    /// Headings above the block.
    pub heading_path: Vec<String>,
    /// The block's text.
    pub text: String,
}

/// The citation URI for a block, shared by the UI, the API and MCP.
#[must_use]
pub fn citation(doc: DocId, block: BlockId) -> String {
    format!("vellum://doc/{doc}#{block}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_names() {
        for kind in DocKind::ALL {
            assert_eq!(kind.as_str().parse::<DocKind>(), Ok(kind));
        }
        for kind in BlockKind::ALL {
            assert_eq!(kind.to_string().parse::<BlockKind>(), Ok(kind));
        }
        assert!("nope".parse::<DocKind>().is_err());
        assert!("nope".parse::<BlockKind>().is_err());
    }

    #[test]
    fn citations_name_the_doc_and_block() {
        let doc = DocId::from_u128(1);
        let block = BlockId::from_u64(1).unwrap_or_else(BlockId::generate);
        assert_eq!(
            citation(doc, block),
            format!("vellum://doc/{doc}#blk-00000001")
        );
    }
}
