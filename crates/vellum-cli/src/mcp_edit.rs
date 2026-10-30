// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Block edits a model proposes through MCP, applied to a page's Vellum
//! Markdown (ADR 0019, D218).
//!
//! Blocks are addressed the way search and `get_blocks` cite them: one
//! per top-level block and one per list item, with the ID in the file or
//! the one derived from the page and the block's position. Every edit is
//! resolved against the page as it was read, so an insert does not shift
//! the blocks a later edit names. Unchanged blocks keep their IDs.

use std::collections::HashMap;

use vellum_format::{Block, Document, List, ListItem, serialize};
use vellum_mcp::ports::{BlockEdit, WorkspaceError};

/// How many citable blocks `doc` has, counted the way `read_document`
/// numbers them: list items one by one, thematic breaks not at all.
fn citable(doc: &Document) -> usize {
    doc.blocks
        .iter()
        .map(|block| match block {
            Block::ThematicBreak => 0,
            Block::List(list) => list.items.len(),
            _ => 1,
        })
        .sum()
}

/// What happens at one block.
#[derive(Debug, Default)]
struct Plan {
    /// `None` keeps the block; `Some(empty)` deletes it.
    replace: Option<Vec<Block>>,
    /// Blocks inserted after it, in order.
    after: Vec<Block>,
}

/// The plan of the block `id`.
fn plan_of<'a>(
    plans: &'a mut [Plan],
    index: &HashMap<&str, usize>,
    id: &str,
) -> Result<&'a mut Plan, WorkspaceError> {
    index
        .get(id)
        .and_then(|k| plans.get_mut(*k))
        .ok_or_else(|| WorkspaceError::Invalid(format!("unknown block {id}")))
}

fn blocks_of(markdown: &str) -> Result<Vec<Block>, WorkspaceError> {
    let blocks = vellum_format::parse(markdown).document.blocks;
    if blocks.is_empty() {
        return Err(WorkspaceError::Invalid(
            "the new Markdown holds no block".into(),
        ));
    }
    Ok(blocks)
}

/// Applies `edits` to the page `markdown`, whose citable blocks have the
/// IDs `ids` in reading order, and returns the new file.
///
/// # Errors
///
/// [`WorkspaceError::Invalid`] for an unknown block, an edit of a block
/// already deleted, or empty Markdown; [`WorkspaceError::Conflict`] when
/// the page changed shape since its blocks were numbered.
pub fn apply(
    markdown: &str,
    ids: &[String],
    edits: &[BlockEdit],
) -> Result<String, WorkspaceError> {
    let parsed = vellum_format::parse(markdown).document;
    if citable(&parsed) != ids.len() {
        return Err(WorkspaceError::Conflict(
            "the page changed while the edit was prepared; read it again"
                .into(),
        ));
    }
    let index: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(k, id)| (id.as_str(), k))
        .collect();
    let mut start = Vec::new();
    let mut plans: Vec<Plan> = ids.iter().map(|_| Plan::default()).collect();
    for edit in edits {
        match edit {
            BlockEdit::Replace { block_id, markdown } => {
                let plan = plan_of(&mut plans, &index, block_id)?;
                if plan.replace.as_ref().is_some_and(Vec::is_empty) {
                    return Err(WorkspaceError::Invalid(format!(
                        "block {block_id} is deleted by an earlier edit"
                    )));
                }
                plan.replace = Some(blocks_of(markdown)?);
            }
            BlockEdit::Delete { block_id } => {
                plan_of(&mut plans, &index, block_id)?.replace =
                    Some(Vec::new());
            }
            BlockEdit::Insert {
                after: None,
                markdown,
            } => start.extend(blocks_of(markdown)?),
            BlockEdit::Insert {
                after: Some(id),
                markdown,
            } => {
                let blocks = blocks_of(markdown)?;
                plan_of(&mut plans, &index, id)?.after.extend(blocks);
            }
        }
    }
    let mut out = Builder {
        blocks: start,
        list: None,
    };
    let mut k = 0;
    for block in &parsed.blocks {
        match block {
            Block::List(list) => {
                out.open(list);
                for item in &list.items {
                    let plan = plans
                        .get_mut(k)
                        .map(std::mem::take)
                        .unwrap_or_default();
                    match plan.replace {
                        None => out.item(item.clone()),
                        Some(blocks) => {
                            blocks.into_iter().for_each(|b| out.push(b))
                        }
                    }
                    plan.after.into_iter().for_each(|b| out.push(b));
                    k += 1;
                }
                out.close();
            }
            Block::ThematicBreak => out.push(block.clone()),
            _ => {
                let plan =
                    plans.get_mut(k).map(std::mem::take).unwrap_or_default();
                match plan.replace {
                    None => out.push(block.clone()),
                    Some(blocks) => {
                        blocks.into_iter().for_each(|b| out.push(b))
                    }
                }
                plan.after.into_iter().for_each(|b| out.push(b));
                k += 1;
            }
        }
    }
    Ok(serialize(&Document {
        front_matter: parsed.front_matter,
        blocks: out.blocks,
    }))
}

/// Collects output blocks, keeping list items of one list together and
/// splitting the list where a non-list block replaces an item.
struct Builder {
    blocks: Vec<Block>,
    /// The list being rebuilt: its shape and the items so far.
    list: Option<(List, Vec<ListItem>)>,
}

impl Builder {
    fn open(&mut self, list: &List) {
        self.list = Some((
            List {
                start: list.start,
                tight: list.tight,
                items: Vec::new(),
            },
            Vec::new(),
        ));
    }

    fn item(&mut self, item: ListItem) {
        if let Some((_, items)) = &mut self.list {
            items.push(item);
        }
    }

    fn flush(&mut self) {
        if let Some((shape, items)) = &mut self.list
            && !items.is_empty()
        {
            self.blocks.push(Block::List(List {
                start: shape.start,
                tight: shape.tight,
                items: std::mem::take(items),
            }));
        }
    }

    fn push(&mut self, block: Block) {
        match block {
            // Items written into a list join it.
            Block::List(list) if self.list.is_some() => {
                list.items.into_iter().for_each(|item| self.item(item));
            }
            other => {
                self.flush();
                self.blocks.push(other);
            }
        }
    }

    fn close(&mut self) {
        self.flush();
        self.list = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C41\n\
        title: Launch plan\nvellum: 1\n---\n\n## Goals ^blk-gxa20001\n\n\
        Ship on 2026-11-24. ^blk-shp20001\n\n- one\n- two\n- three\n";

    fn ids() -> Vec<String> {
        let doc = vellum_private::read_document(
            vellum_kernel::SpaceId::from_u128(1),
            "eng/launch.md",
            PAGE.to_owned(),
            vellum_kernel::Timestamp::now(),
        )
        .unwrap();
        doc.blocks.iter().map(|b| b.id.to_string()).collect()
    }

    #[test]
    fn replaces_inserts_and_deletes_by_block_id() {
        let ids = ids();
        assert_eq!(ids.len(), 5);
        let out = apply(
            PAGE,
            &ids,
            &[
                BlockEdit::Replace {
                    block_id: "blk-shp20001".into(),
                    markdown: "Ship on 2026-12-01.".into(),
                },
                BlockEdit::Insert {
                    after: Some("blk-gxa20001".into()),
                    markdown: "Owner: Priya.".into(),
                },
                BlockEdit::Delete {
                    block_id: ids[3].clone(),
                },
                BlockEdit::Insert {
                    after: Some(ids[4].clone()),
                    markdown: "- four".into(),
                },
            ],
        )
        .unwrap();
        assert!(
            out.starts_with("---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C41\n"),
            "{out}"
        );
        assert!(
            out.contains(
                "## Goals ^blk-gxa20001\n\nOwner: Priya.\n\nShip on 2026-12-01."
            ),
            "{out}"
        );
        assert!(out.contains("- one\n- three\n- four"), "{out}");
        assert!(!out.contains("two"), "{out}");
    }

    #[test]
    fn unknown_blocks_and_stale_pages_are_refused() {
        let ids = ids();
        let unknown = apply(
            PAGE,
            &ids,
            &[BlockEdit::Delete {
                block_id: "blk-nope0000".into(),
            }],
        );
        assert!(matches!(unknown, Err(WorkspaceError::Invalid(_))));
        let stale = apply(PAGE, &ids[..2], &[]);
        assert!(matches!(stale, Err(WorkspaceError::Conflict(_))));
        let empty = apply(
            PAGE,
            &ids,
            &[BlockEdit::Insert {
                after: None,
                markdown: "  ".into(),
            }],
        );
        assert!(matches!(empty, Err(WorkspaceError::Invalid(_))));
    }

    #[test]
    fn a_paragraph_replacing_an_item_splits_the_list() {
        let ids = ids();
        let out = apply(
            PAGE,
            &ids,
            &[BlockEdit::Replace {
                block_id: ids[3].clone(),
                markdown: "Not a list item.".into(),
            }],
        )
        .unwrap();
        assert!(
            out.contains("- one\n\nNot a list item.\n\n- three"),
            "{out}"
        );
    }
}
