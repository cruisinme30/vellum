// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Block IDs on every block type, and the task item `{…}` field.

use vellum_format::ast::{Block, Inline};
use vellum_format::{parse, serialize};

fn round_trips(src: &str) {
    let doc = parse(src).document;
    let canonical = serialize(&doc);
    assert_eq!(parse(&canonical).document, doc, "{canonical}");
    assert_eq!(serialize(&parse(&canonical).document), canonical);
}

#[test]
fn task_attributes_become_a_field() {
    let doc =
        parse("- [ ] Call @priya {due=\"2026-11-02\" priority=high} ^blk-a1b2c3d4\n").document;
    let Some(Block::List(list)) = doc.blocks.first() else {
        panic!("not a list: {doc:?}");
    };
    let item = &list.items[0];
    assert_eq!(
        item.attrs.pairs,
        [
            ("due".to_owned(), "2026-11-02".to_owned()),
            ("priority".to_owned(), "high".to_owned()),
        ]
    );
    let Some(Block::Paragraph { content, id }) = item.blocks.first() else {
        panic!("no paragraph: {item:?}");
    };
    assert_eq!(id.map(|id| id.to_string()).as_deref(), Some("blk-a1b2c3d4"));
    assert!(
        !content
            .iter()
            .any(|i| matches!(i, Inline::Text { text } if text.contains('{'))),
        "attrs left in the text: {content:?}"
    );
    assert_eq!(
        serialize(&doc),
        "- [ ] Call @priya {due=\"2026-11-02\" priority=\"high\"} ^blk-a1b2c3d4\n"
    );
}

#[test]
fn task_attribute_edge_cases_round_trip() {
    for src in [
        "- [ ] {#id .c k=\"v\"}\n",
        "- [ ] \n{ }\n# x\n",
        "- [ ] Northstar \\\n{#id}\n",
        "- [ ] Ship {due=\"2026-11-02\"} ^blk-b0000006 { }\n",
        "- [ ] Literal braces {}\n",
    ] {
        round_trips(src);
    }
}

#[test]
fn ids_survive_on_every_block_type() {
    let src = "> Quote\n\n^blk-a0000001\n\n| a |\n| - |\n| 1 |\n\n^blk-a0000002\n\n\
               ```rust ^blk-a0000003\nfn main() {}\n```\n\n$$ ^blk-a0000004\nx\n$$\n\n\
               ::: details Why ^blk-a0000005 {.wide}\n\nBody\n\n:::\n";
    let doc = parse(src).document;
    let ids: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::BlockQuote { id, .. } | Block::Math { id, .. } => *id,
            Block::Table(t) => t.id,
            Block::Fence(f) => f.id,
            Block::Container(c) => c.id,
            _ => None,
        })
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "blk-a0000001",
            "blk-a0000002",
            "blk-a0000003",
            "blk-a0000004",
            "blk-a0000005"
        ]
    );
    round_trips(src);
}

#[test]
fn empty_first_column_survives() {
    round_trips("::: columns\n---\n---\n:::\n");
}
