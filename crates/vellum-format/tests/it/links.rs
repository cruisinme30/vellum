// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Links and images whose text, destination or title hold characters that
//! Markdown link syntax uses: the canonical text reads back as the same
//! document.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use vellum_format::ast::{Block, Inline};
use vellum_format::{Document, parse, serialize};

fn text(t: &str) -> Inline {
    Inline::Text { text: t.into() }
}

fn paragraph(content: Vec<Inline>) -> Document {
    Document {
        front_matter: None,
        blocks: vec![Block::Paragraph { content, id: None }],
    }
}

fn link(url: &str, title: &str, content: Vec<Inline>) -> Inline {
    Inline::Link {
        url: url.into(),
        title: title.into(),
        content,
    }
}

/// Writes `doc` and asserts that it reads back unchanged and that its
/// text is a fixed point.
fn assert_round_trips(doc: &Document) {
    let text = serialize(doc);
    let read = parse(&text).document;
    assert_eq!(&read, doc, "written as {text:?}");
    assert_eq!(serialize(&read), text, "not idempotent");
}

/// Asserts the fuzz property for `src`.
fn assert_fixed_point(src: &str) {
    let parsed = parse(src).document;
    let text = serialize(&parsed);
    let again = parse(&text).document;
    assert_eq!(again, parsed, "{src:?} was written as {text:?}");
    assert_eq!(serialize(&again), text, "{src:?}: not idempotent");
}

const DESTINATIONS: &[&str] = &[
    "docs/getting started.md",
    "a(b)c",
    "a(b",
    "a)b",
    "a\"b",
    "a'b",
    "<a>",
    "a\\b",
    "a&amp;b",
    "a b\tc",
    "https://example.com/?q=a b",
];

const TITLES: &[&str] = &[
    "",
    "Deploy runbook",
    "say \"hi\"",
    "it's",
    "(paren)",
    "a\\b",
    "&amp;",
];

const LABELS: &[&str] = &[
    "the docs",
    "\"Deploy runbook\")",
    "a](b)",
    "(x)",
    "<b>",
    "x \\ y",
    "https://example.com/status",
    "www.example.com",
    "ann@example.com",
    "a\0b",
];

#[test]
fn destinations_titles_and_labels_round_trip() {
    for url in DESTINATIONS {
        for title in TITLES {
            for label in LABELS {
                assert_round_trips(&paragraph(vec![
                    text("See "),
                    link(url, title, vec![text(label)]),
                    text("."),
                ]));
            }
        }
    }
}

#[test]
fn autolinks_inside_link_text_stay_links() {
    // Fuzzer finding (`format_roundtrip`): `<https://…>` inside link text
    // is a link, but it was written bare, and the autolink extension does
    // not run inside link text.
    let inner = |url: &str| link(url, "", vec![text(url)]);
    for content in [
        vec![inner("https://example.com/status")],
        vec![
            text("see "),
            inner("https://example.com/status"),
            text(" now"),
        ],
        vec![
            text("mail "),
            link("mailto:ann@example.com", "", vec![text("ann@example.com")]),
        ],
    ] {
        assert_round_trips(&paragraph(vec![
            text("See "),
            link("docs/getting started.md", "", content),
            text("."),
        ]));
    }
}

#[test]
fn fuzz_reproducer_round_trips() {
    assert_fixed_point(concat!(
        "See [the deploy runboo \"Deploy runbook\"), <https://northstar.dev/status> ",
        "or https://northstav.dev/changelo\0\0hcs](<docs/getting started.md>).\n\n",
        "![Architwww.re overView]k](https://northstar.dev/runbooks/deploy \"Deploy runbook\"), ",
        "<https://northstar.r.dev, or open [the docs](<dost path\")\n",
    ));
    for src in [
        "[a <https://example.com> b](<x y>)\n",
        "![a <https://example.com> b](x)\n",
        "[a https://example.com b](x)\n",
        "[a www.example.com b](x)\n",
        "[a ann@example.com b](x)\n",
        "[a](<b c> \"t (u)\")\n",
        "[a](b(c)d)\n",
        "[a](<b)c>)\n",
    ] {
        assert_fixed_point(src);
    }
}
