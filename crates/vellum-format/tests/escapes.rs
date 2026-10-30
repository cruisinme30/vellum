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

#[test]
fn an_escaped_character_does_not_end_an_extra() {
    // Fuzzer finding (`format_roundtrip`): `@priyah@ip` is text, but its
    // canonical form `@priyah\@ip` read back as a mention.
    let doc = parse("Call @priyah@ip now\n").document;
    assert_eq!(doc, paragraph("Call @priyah@ip now"));
    assert_eq!(parse(&serialize(&doc)).document, doc);
    for src in [
        "Call @priyah\\@ip now\n",
        "@ann\\-b\n",
        "@today\\-x\n",
        "2026-11-24\\-x\n",
        "ENG-142\\-2\n",
    ] {
        let doc = parse(src).document;
        let canonical = serialize(&doc);
        assert_eq!(parse(&canonical).document, doc, "{src:?} as {canonical:?}");
        assert_eq!(serialize(&parse(&canonical).document), canonical);
    }
}

#[test]
fn text_starting_with_a_byte_order_mark_keeps_it() {
    // Fuzzer finding (`format_roundtrip`): only a leading byte order mark is
    // dropped, but text that starts with one was written at the file start.
    let doc = parse(" \u{feff}^a\n").document;
    assert_eq!(doc, paragraph("\u{feff}^a"));
    let canonical = serialize(&doc);
    assert_eq!(canonical, "&#65279;\\^a\n");
    assert_eq!(parse(&canonical).document, doc);
}

#[test]
fn code_with_a_long_backtick_run_round_trips() {
    // Fuzzer finding (`format_roundtrip`): the parser reads code spans with
    // fences of at most 80 backticks, so a fence one longer than an 89-long
    // run in the code was read as text.
    let long = "`".repeat(89);
    let src = format!("``a {long} b``\n");
    let doc = parse(&src).document;
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph {
            content: vec![Inline::Code {
                code: format!("a {long} b"),
            }],
            id: None,
        }]
    );
    let canonical = serialize(&doc);
    assert_eq!(
        canonical,
        src.replacen("``", "`", 1).replacen(" b``", " b`", 1)
    );
    assert_eq!(parse(&canonical).document, doc);
}
