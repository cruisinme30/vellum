// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! From an opened page to blocks that can be indexed and quoted.

use vellum_format::{Block, Document, ListItem, plain_text, serialize};
use vellum_kernel::{BlockId, DocId, SpaceId, Timestamp};
use vellum_search::{BlockKind, DocKind, IndexBlock, IndexDocument};

/// One block of a private page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateBlock {
    /// Its ID: the one in the file, or a stable one derived from the
    /// document and the block's position when the file has none.
    pub id: BlockId,
    /// What kind of block it is.
    pub kind: BlockKind,
    /// The heading that opens its section, if any.
    pub section: Option<BlockId>,
    /// Heading texts from the top of the page down to its section.
    pub heading_path: Vec<String>,
    /// Plain text, formatting stripped.
    pub text: String,
    /// The block as Vellum Markdown.
    pub markdown: String,
}

/// A private page, opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateDoc {
    /// The page's ID, from its front matter.
    pub id: DocId,
    /// The space it belongs to.
    pub space: SpaceId,
    /// Its title: the front-matter title, else the first heading, else the
    /// file name.
    pub title: String,
    /// Its path in the store.
    pub path: String,
    /// The whole file.
    pub markdown: String,
    /// When the file last changed.
    pub modified: Timestamp,
    /// Its blocks, in reading order.
    pub blocks: Vec<PrivateBlock>,
}

/// Why a file could not be read as a page.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtractError {
    /// The front matter has no valid `id`.
    #[error("{0} has no id in its front matter")]
    NoId(String),
}

impl PrivateDoc {
    /// The page as the search index sees it.
    #[must_use]
    pub fn to_index(&self) -> IndexDocument {
        IndexDocument {
            space: self.space,
            doc: self.id,
            kind: DocKind::Page,
            title: self.title.clone(),
            path: self.path.clone(),
            owner: None,
            modified: self.modified,
            ancestors: vec![self.space.into()],
            trashed: false,
            blocks: self
                .blocks
                .iter()
                .map(|b| IndexBlock {
                    id: b.id,
                    kind: b.kind,
                    section: b.section,
                    heading_path: b.heading_path.clone(),
                    text: b.text.clone(),
                })
                .collect(),
        }
    }
}

/// Reads an opened page.
///
/// # Errors
///
/// [`ExtractError::NoId`] when the front matter has no valid `id`: identity
/// is the front-matter ID, never the path.
pub fn read_document(
    space: SpaceId,
    path: &str,
    markdown: String,
    modified: Timestamp,
) -> Result<PrivateDoc, ExtractError> {
    let parsed = vellum_format::parse(&markdown);
    let front = parsed.document.front_matter.as_ref();
    let id: DocId = front
        .and_then(vellum_format::FrontMatter::id)
        .ok_or_else(|| ExtractError::NoId(path.to_owned()))?
        .into();
    let mut walker = Walker {
        doc: id,
        headings: Vec::new(),
        blocks: Vec::new(),
    };
    for block in &parsed.document.blocks {
        walker.top(block);
    }
    let title = front
        .and_then(vellum_format::FrontMatter::title)
        .map(str::to_owned)
        .or_else(|| {
            walker
                .blocks
                .iter()
                .find(|b| b.kind == BlockKind::Heading)
                .map(|b| b.text.clone())
        })
        .unwrap_or_else(|| file_stem(path).to_owned());
    Ok(PrivateDoc {
        id,
        space,
        title,
        path: path.to_owned(),
        markdown,
        modified,
        blocks: walker.blocks,
    })
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

struct Walker {
    doc: DocId,
    /// Open headings: level, ID, text.
    headings: Vec<(u8, BlockId, String)>,
    blocks: Vec<PrivateBlock>,
}

impl Walker {
    fn top(&mut self, block: &Block) {
        match block {
            Block::ThematicBreak => {}
            Block::Heading {
                level, content, id, ..
            } => {
                let id = self.id_or_derived(*id);
                let text = plain_text(content);
                self.headings.retain(|(l, _, _)| l < level);
                self.headings.push((*level, id, text.clone()));
                self.push(id, BlockKind::Heading, text, block);
            }
            Block::List(list) => {
                for item in &list.items {
                    let kind = if item.task.is_some() {
                        BlockKind::Task
                    } else {
                        BlockKind::ListItem
                    };
                    let id = item.attrs.id.as_deref().and_then(|s| s.parse().ok());
                    let id = self.id_or_derived(id);
                    let one = Block::List(vellum_format::List {
                        start: list.start,
                        tight: list.tight,
                        items: vec![item.clone()],
                    });
                    self.push(id, kind, item_text(item), &one);
                }
            }
            Block::Paragraph { content, id } => {
                let id = self.id_or_derived(*id);
                self.push(id, BlockKind::Paragraph, plain_text(content), block);
            }
            Block::BlockQuote { blocks, id } => {
                let id = self.id_or_derived(*id);
                self.push(id, BlockKind::Quote, texts(blocks), block);
            }
            Block::Fence(fence) => {
                let kind = if fence.kind() == vellum_format::FenceKind::Code {
                    BlockKind::Code
                } else {
                    BlockKind::Other
                };
                let id = self.id_or_derived(fence.id);
                self.push(id, kind, fence.literal.trim_end().to_owned(), block);
            }
            Block::Math { literal, id } => {
                let id = self.id_or_derived(*id);
                self.push(id, BlockKind::Other, literal.trim().to_owned(), block);
            }
            Block::Table(table) => {
                let id = self.id_or_derived(table.id);
                self.push(id, BlockKind::Table, text_of(block), block);
            }
            Block::Container(container) => {
                let id = self.id_or_derived(container.id);
                self.push(id, BlockKind::Callout, text_of(block), block);
            }
        }
    }

    fn push(&mut self, id: BlockId, kind: BlockKind, text: String, block: &Block) {
        let markdown = serialize(&Document {
            front_matter: None,
            blocks: vec![block.clone()],
        })
        .trim_end()
        .to_owned();
        self.blocks.push(PrivateBlock {
            id,
            kind,
            section: self.headings.last().map(|(_, id, _)| *id),
            heading_path: self.headings.iter().map(|(_, _, t)| t.clone()).collect(),
            text,
            markdown,
        });
    }

    /// The block's own ID, or one derived from the page and the block's
    /// position, so citations stay the same across restarts.
    fn id_or_derived(&self, id: Option<BlockId>) -> BlockId {
        id.unwrap_or_else(|| {
            let mut hash = Fnv::new();
            hash.write(&self.doc.as_u128().to_le_bytes());
            hash.write(&self.blocks.len().to_le_bytes());
            BlockId::from_u64(hash.finish() & BlockId::MAX).unwrap_or_else(BlockId::generate)
        })
    }
}

/// FNV-1a: tiny, and stable across Rust versions, unlike `DefaultHasher`.
struct Fnv(u64);

impl Fnv {
    const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    const fn finish(&self) -> u64 {
        self.0
    }
}

fn item_text(item: &ListItem) -> String {
    texts(&item.blocks)
}

fn texts(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(text_of)
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain text of any block, nested blocks included.
fn text_of(block: &Block) -> String {
    match block {
        Block::ThematicBreak => String::new(),
        Block::Paragraph { content, .. } | Block::Heading { content, .. } => plain_text(content),
        Block::BlockQuote { blocks, .. } => texts(blocks),
        Block::List(list) => list
            .items
            .iter()
            .map(item_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Fence(fence) => fence.literal.trim_end().to_owned(),
        Block::Math { literal, .. } => literal.trim().to_owned(),
        Block::Table(table) => std::iter::once(&table.header)
            .chain(&table.rows)
            .map(|row| {
                row.iter()
                    .map(|c| plain_text(c))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Container(container) => container
            .sections
            .iter()
            .map(|section| texts(section))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\nid: 01JDPR7A00000000000000000A\ntitle: Board notes\nvellum: 1\n---\n\n# Q4 plan ^blk-p1an0001\n\nRaise the seed round in December. ^blk-ra1se001\n\n- [ ] Draft the deck\n- [x] Pick the lead\n\n## Risks\n\n> Runway is nine months.\n\n| Item | Cost |\n| --- | --- |\n| Legal | 40k |\n";

    fn read() -> PrivateDoc {
        read_document(
            SpaceId::from_u128(7),
            "drives/shared/board/notes.md",
            PAGE.to_owned(),
            Timestamp::now(),
        )
        .unwrap()
    }

    #[test]
    fn pages_become_blocks_with_sections_and_text() {
        let doc = read();
        assert_eq!(doc.title, "Board notes");
        assert_eq!(doc.blocks[0].id.to_string(), "blk-p1an0001");
        assert_eq!(doc.blocks[1].id.to_string(), "blk-ra1se001");
        let kinds: Vec<_> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading,
                BlockKind::Paragraph,
                BlockKind::Task,
                BlockKind::Task,
                BlockKind::Heading,
                BlockKind::Quote,
                BlockKind::Table,
            ]
        );
        let quote = &doc.blocks[5];
        assert_eq!(quote.heading_path, vec!["Q4 plan", "Risks"]);
        assert_eq!(quote.section, Some(doc.blocks[4].id));
        assert_eq!(quote.text, "Runway is nine months.");
        assert!(doc.blocks[6].text.contains("Legal | 40k"));
        assert!(doc.blocks[2].markdown.starts_with("- [ ] Draft the deck"));
    }

    #[test]
    fn derived_block_ids_are_stable() {
        let (a, b) = (read(), read());
        let ids = |d: &PrivateDoc| d.blocks.iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
        let unique: std::collections::HashSet<_> = ids(&a).into_iter().collect();
        assert_eq!(unique.len(), a.blocks.len());
    }

    #[test]
    fn a_page_without_an_id_is_refused_and_titles_fall_back() {
        let err = read_document(
            SpaceId::from_u128(7),
            "a/untitled.md",
            "# Hello\n".to_owned(),
            Timestamp::now(),
        )
        .unwrap_err();
        assert_eq!(err, ExtractError::NoId("a/untitled.md".to_owned()));

        let no_title = "---\nid: 01JDPR7A00000000000000000B\n---\n\nJust text.\n";
        let doc = read_document(
            SpaceId::from_u128(7),
            "a/plain.md",
            no_title.to_owned(),
            Timestamp::now(),
        )
        .unwrap();
        assert_eq!(doc.title, "plain");
        assert_eq!(doc.to_index().blocks.len(), 1);
    }
}
