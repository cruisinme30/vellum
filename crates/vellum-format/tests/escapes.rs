// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A backslash keeps a would-be mention, date, issue key or block ID as text.

use vellum_format::ast::{Block, Inline};
use vellum_format::{Document, parse, serialize};

fn paragraph(text: &str) -> Document {
    Document {
        front_matter: None,
        blocks: vec![Block::Paragraph {
            content: vec![Inline::Text { text: text.into() }],
            id: None,
        }],
    }
}

#[test]
fn escaped_extras_are_literal_text() {
    let doc = parse("Ping \\@everyone about ENG\\-142 by 2026\\-11-24 \\^blk-a1b2c3d4\n").document;
    assert_eq!(
        doc,
        paragraph("Ping @everyone about ENG-142 by 2026-11-24 ^blk-a1b2c3d4")
    );
}

#[test]
fn literal_text_is_written_with_escapes() {
    let doc = paragraph("@everyone, ENG-142, 2026-11-24 and @today");
    let canonical = serialize(&doc);
    assert_eq!(
        canonical,
        "\\@everyone, ENG\\-142, 2026\\-11-24 and \\@today\n"
    );
    assert_eq!(parse(&canonical).document, doc);
}

#[test]
fn unescaped_extras_are_still_recognised() {
    let doc = parse("Ping @priya about ENG-142\n").document;
    let Some(Block::Paragraph { content, .. }) = doc.blocks.first() else {
        panic!("{doc:?}");
    };
    assert!(content.iter().any(|i| matches!(i, Inline::Mention { .. })));
    assert!(content.iter().any(|i| matches!(i, Inline::IssueKey { .. })));
}
