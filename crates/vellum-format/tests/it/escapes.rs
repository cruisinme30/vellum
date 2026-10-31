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
    let doc = parse(
        "Ping \\@everyone about ENG\\-142 by 2026\\-11-24 \\^blk-a1b2c3d4\n",
    )
    .document;
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

#[test]
fn wiki_link_targets_and_labels_keep_brackets_bars_and_ampersands() {
    // Fuzzer finding (`format_roundtrip`): `[[Runbook \\[draft]]` has the
    // target `Runbook [draft`, which was written back without its escape.
    for src in [
        "Read [[Runbook \\[draft]] first\n",
        "Read [[a\\]b|c \\| d]] first\n",
        "Read [[R&amp;D|Q&amp;A]] first\n",
        "Read [[back\\\\slash]] first\n",
    ] {
        let doc = parse(src).document;
        assert!(
            matches!(
                &doc.blocks[..],
                [
                    Block::Paragraph { content, .. }
                ] if matches!(content.get(1), Some(Inline::WikiLink { .. }))
            ),
            "{src:?}: {doc:?}"
        );
        let canonical = serialize(&doc);
        assert_eq!(parse(&canonical).document, doc, "{src:?} as {canonical:?}");
    }
}

#[test]
fn a_quoted_line_after_a_vertical_tab_stays_paragraph_text() {
    // Fuzzer finding (`format_roundtrip`): in `> A\n\u{b}---`, the second
    // line is a lazy continuation of the quoted paragraph. Written back as
    // `> \u{b}---` it became a one-column table, because the table
    // extension trims the vertical tab before a delimiter row.
    for src in [
        "> A\n\u{b}---\n",
        "> A\n\u{c}---\n",
        "> A\n\u{b}:--\n",
        "> A\n\u{b}\u{c}- b\n",
    ] {
        let doc = parse(src).document;
        let canonical = serialize(&doc);
        let again = parse(&canonical).document;
        assert_eq!(again, doc, "{src:?} as {canonical:?}");
        assert_eq!(serialize(&again), canonical, "{src:?} is not idempotent");
    }
}

#[test]
fn the_nightly_quote_and_table_crash_round_trips() {
    // The input the nightly fuzz run of 2026-10-30 failed on.
    let src = concat!(
        "> A pnno::: inr\u{0}\u{0}nr\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}",
        "\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}",
        "\u{0}\u{0}\u{0}o\u{1}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0} Policy c",
        "hangzes ship blind.\n\u{b}---\n\n**Outc\nEvery \u{0}\u{0}",
        "\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}",
        "\u{0}\u{0}\u{0}\u{0}\u{0}o\u{1}\u{0}\u{0}\u{0}\u{0}\u{0}",
        "\u{0} Policy changzes ship blind.\n\u{b}---```calculator\nun",
        "its: tUnits = 100 [0..1000 step 10]\nprice: Pric\n\n**Outc\n",
        "Every change shipe = 12\ntotal: Ts with  sh:::\no",
    );
    let doc = parse(src).document;
    let canonical = serialize(&doc);
    assert_eq!(parse(&canonical).document, doc, "as {canonical:?}");
}

#[test]
fn a_wiki_link_across_setext_heading_lines_round_trips() {
    // Fuzzer finding (`format_roundtrip`, nightly of 2026-10-30): a wiki
    // link spanning the lines of a setext heading kept its line breaks,
    // which ended the ATX heading it was written back as.
    for src in [
        "Read [[Run\nbook]] first\n-\n",
        "Read [[a\n*u \n-- - *]] be\n---\n",
        "[[Run\nbook|the\nbook]]\n===\n",
    ] {
        let doc = parse(src).document;
        assert!(
            matches!(&doc.blocks[..], [Block::Heading { .. }]),
            "{src:?}: {doc:?}"
        );
        let canonical = serialize(&doc);
        let again = parse(&canonical).document;
        assert_eq!(again, doc, "{src:?} as {canonical:?}");
        assert_eq!(serialize(&again), canonical, "{src:?} is not idempotent");
    }
}

#[test]
fn math_with_a_fence_line_keeps_it_literal() {
    // Fuzzer finding (`format_roundtrip`): math whose text holds a code
    // or container fence line was written as a `$$` block, which the
    // container pass does not skip, so the line opened a code fence or
    // closed a container around it when read back.
    for src in [
        "```math\na\n```x\n```\n\n> [!WARNING] T\n> b\n",
        "````math\na\n~~~\n````\n\n> [!NOTE] T\n> b\n",
        "```math\na\n::::\nb\n```\n",
        "::: info\n\n```math\na\n:::\nb\n```\n\n:::\n",
        ":::: columns\n\n::: stats\n\n```math\n::::\n```\n\n:::\n\n::::\n",
    ] {
        let doc = parse(src).document;
        let canonical = serialize(&doc);
        let again = parse(&canonical).document;
        assert_eq!(again, doc, "{src:?} as {canonical:?}");
        assert_eq!(serialize(&again), canonical, "{src:?} is not idempotent");
    }
}

#[test]
fn fuzzer_findings_of_2026_10_30_round_trip() {
    // Local format_roundtrip runs: blank control characters at a cell's or
    // paragraph's edge, autolinks the extension does not read bare, and
    // `>` runs after text.
    for src in [
        "\u{b}\n\u{b}-\n",
        "\u{c}\n\u{c}-\n",
        "\u{b}\n:-\n",
        "n||\n-|-\n\u{c}\n",
        "\u{b}\n^blk-a0000004\n",
        ".\n\u{b}^blk-a0000003\n",
        "<t@F>\n",
        "<http://>\n",
        "<http://#s>\n",
        "t>>>>>>>>>\\>>>>>>>>>>>>>>>>>>>>>>>>\n",
        "$$\n```\n$$\n#>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
    ] {
        let doc = parse(src).document;
        let canonical = serialize(&doc);
        let again = parse(&canonical).document;
        assert_eq!(again, doc, "{src:?} as {canonical:?}");
        assert_eq!(serialize(&again), canonical, "{src:?} is not idempotent");
    }
}

#[test]
fn a_long_gt_run_after_text_is_kept_whole() {
    let src = format!("t{}\n", ">".repeat(40));
    let doc = parse(&src).document;
    let whole = vec![Inline::Text {
        text: src.trim_end().to_owned(),
    }];
    assert!(
        matches!(
            &doc.blocks[..],
            [Block::Paragraph { content, .. }] if *content == whole
        ),
        "{doc:?}"
    );
}
