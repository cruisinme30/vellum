// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Turns a saved Markdown page into the blocks the index stores.
//!
//! The walk follows the format AST in reading order. Headings open
//! sections, so every block records the heading it sits under and the
//! heading texts above it; that is what search shows as a hit's context.
//! Blocks keep the `^blk-…` IDs written in the file. A block written
//! without one (a file typed by hand) gets an ID derived from the document
//! and its position, which is stable until blocks are inserted above it.

use vellum_format::{Block, ContainerKind, FenceKind, Inline, plain_text};
use vellum_kernel::{BlockId, DocId};

use crate::model::{BlockKind, IndexBlock};

/// The blocks of a Markdown page, in reading order.
#[must_use]
pub fn blocks(doc: DocId, src: &str) -> Vec<IndexBlock> {
    let parsed = vellum_format::parse(src);
    let mut walker = Walker {
        doc,
        ordinal: 0,
        headings: Vec::new(),
        out: Vec::new(),
    };
    walker.walk(&parsed.document.blocks);
    walker.out
}

struct Walker {
    doc: DocId,
    ordinal: u64,
    /// The open headings: level, text and ID, outermost first.
    headings: Vec<(u8, String, BlockId)>,
    out: Vec<IndexBlock>,
}

impl Walker {
    fn walk(&mut self, blocks: &[Block]) {
        for block in blocks {
            self.block(block);
        }
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Paragraph { content, id } => {
                self.emit(*id, BlockKind::Paragraph, plain_text(content));
            }
            Block::Heading {
                level, content, id, ..
            } => self.heading(*level, plain_text(content), *id),
            Block::BlockQuote { blocks, id } => {
                self.emit(*id, BlockKind::Quote, text_of(blocks));
            }
            Block::List(list) => {
                for item in &list.items {
                    let first = item.blocks.iter().find_map(|b| match b {
                        Block::Paragraph { id, .. } => Some(*id),
                        _ => None,
                    });
                    let text =
                        join(item.blocks.iter().filter_map(|b| match b {
                            Block::Paragraph { content, .. } => {
                                Some(plain_text(content))
                            }
                            _ => None,
                        }));
                    let kind = if item.task.is_some() {
                        BlockKind::Task
                    } else {
                        BlockKind::ListItem
                    };
                    self.emit(first.flatten(), kind, text);
                    for nested in &item.blocks {
                        if !matches!(nested, Block::Paragraph { .. }) {
                            self.block(nested);
                        }
                    }
                }
            }
            Block::Fence(fence) => {
                let kind = match fence.kind() {
                    FenceKind::Code => BlockKind::Code,
                    FenceKind::Table => BlockKind::Table,
                    _ => BlockKind::Other,
                };
                self.emit(fence.id, kind, fence.literal.trim_end().to_owned());
            }
            Block::Table(table) => {
                let rows = std::iter::once(&table.header)
                    .chain(&table.rows)
                    .map(|row| row_text(row));
                self.emit(table.id, BlockKind::Table, join(rows));
            }
            Block::Container(container) => match container.kind() {
                ContainerKind::Callout | ContainerKind::Details => {
                    let body = container.sections.iter().map(|s| text_of(s));
                    let text = join(
                        std::iter::once(container.arg.clone()).chain(body),
                    );
                    self.emit(container.id, BlockKind::Callout, text);
                }
                _ => {
                    for section in &container.sections {
                        self.walk(section);
                    }
                }
            },
            Block::ThematicBreak | Block::Math { .. } => {}
        }
    }

    fn heading(&mut self, level: u8, text: String, id: Option<BlockId>) {
        let id = self.id(id);
        while self.headings.last().is_some_and(|(l, _, _)| *l >= level) {
            let _ = self.headings.pop();
        }
        self.headings.push((level, text.clone(), id));
        self.out.push(IndexBlock {
            id,
            kind: BlockKind::Heading,
            section: Some(id),
            heading_path: self.path(),
            text,
        });
    }

    fn emit(&mut self, id: Option<BlockId>, kind: BlockKind, text: String) {
        let id = self.id(id);
        if text.trim().is_empty() {
            return;
        }
        self.out.push(IndexBlock {
            id,
            kind,
            section: self.headings.last().map(|(_, _, id)| *id),
            heading_path: self.path(),
            text,
        });
    }

    fn path(&self) -> Vec<String> {
        self.headings.iter().map(|(_, t, _)| t.clone()).collect()
    }

    /// The written ID, or one derived from the document and position.
    fn id(&mut self, given: Option<BlockId>) -> BlockId {
        self.ordinal += 1;
        given.unwrap_or_else(|| derived(self.doc, self.ordinal))
    }
}

/// FNV-1a over the document ID and the block's position, cut to 40 bits.
fn derived(doc: DocId, ordinal: u64) -> BlockId {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in doc
        .as_u128()
        .to_le_bytes()
        .into_iter()
        .chain(ordinal.to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    BlockId::from_u64(hash & BlockId::MAX).unwrap_or_else(BlockId::generate)
}

fn row_text(cells: &[Vec<Inline>]) -> String {
    cells
        .iter()
        .map(|cell| plain_text(cell))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Every piece of text inside `blocks`, one line per block.
fn text_of(blocks: &[Block]) -> String {
    join(blocks.iter().map(|block| {
        match block {
            Block::Paragraph { content, .. }
            | Block::Heading { content, .. } => plain_text(content),
            Block::BlockQuote { blocks, .. } => text_of(blocks),
            Block::List(list) => {
                join(list.items.iter().map(|item| text_of(&item.blocks)))
            }
            Block::Fence(fence) => fence.literal.trim_end().to_owned(),
            Block::Table(table) => join(
                std::iter::once(&table.header)
                    .chain(&table.rows)
                    .map(|row| row_text(row)),
            ),
            Block::Container(container) => {
                join(container.sections.iter().map(|s| text_of(s)))
            }
            Block::ThematicBreak | Block::Math { .. } => String::new(),
        }
    }))
}

fn join(parts: impl Iterator<Item = String>) -> String {
    parts
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "\
---
title: Launch plan
---

Intro before any heading.

# Launch

The launch is on Friday. ^blk-00000001

## Risks

- Payments may lag
- [ ] Ask finance about refunds

> Ship small, ship often.

| Owner | Task |
| ----- | ---- |
| Ada   | Docs |

```rust
fn main() {}
```

::: warn
Freeze the schema first.
:::

# Later

Notes.
";

    fn kinds(blocks: &[IndexBlock]) -> Vec<BlockKind> {
        blocks.iter().map(|b| b.kind).collect()
    }

    #[test]
    fn walks_blocks_in_reading_order_with_sections() {
        let doc = DocId::from_u128(7);
        let blocks = blocks(doc, PAGE);
        assert_eq!(
            kinds(&blocks),
            vec![
                BlockKind::Paragraph,
                BlockKind::Heading,
                BlockKind::Paragraph,
                BlockKind::Heading,
                BlockKind::ListItem,
                BlockKind::Task,
                BlockKind::Quote,
                BlockKind::Table,
                BlockKind::Code,
                BlockKind::Callout,
                BlockKind::Heading,
                BlockKind::Paragraph,
            ]
        );
        assert_eq!(blocks[0].section, None);
        assert!(blocks[0].heading_path.is_empty());

        let launch = &blocks[1];
        assert_eq!(launch.section, Some(launch.id));
        assert_eq!(launch.heading_path, vec!["Launch"]);

        let friday = &blocks[2];
        assert_eq!(friday.id, BlockId::from_u64(1).unwrap());
        assert_eq!(friday.section, Some(launch.id));

        let risks = &blocks[3];
        let task = &blocks[5];
        assert_eq!(task.text, "Ask finance about refunds");
        assert_eq!(task.section, Some(risks.id));
        assert_eq!(task.heading_path, vec!["Launch", "Risks"]);

        assert_eq!(blocks[7].text, "Owner | Task\nAda | Docs");
        assert_eq!(blocks[8].text, "fn main() {}");
        assert_eq!(blocks[9].text, "Freeze the schema first.");

        let later = &blocks[10];
        assert_eq!(later.heading_path, vec!["Later"]);
        assert_eq!(blocks[11].section, Some(later.id));
    }

    #[test]
    fn derived_ids_are_stable_and_distinct() {
        let doc = DocId::from_u128(9);
        let first = blocks(doc, PAGE);
        let again = blocks(doc, PAGE);
        assert_eq!(first, again);
        let mut ids: Vec<_> = first.iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), first.len());
        let other = blocks(DocId::from_u128(10), PAGE);
        assert_ne!(first[0].id, other[0].id);
    }

    #[test]
    fn empty_and_hostile_input_yield_no_panic() {
        assert!(blocks(DocId::from_u128(1), "").is_empty());
        let deep = format!("{}deep", "> ".repeat(500));
        let _ = blocks(DocId::from_u128(1), &deep);
    }
}
