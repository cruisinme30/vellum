// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Mark delimiters inside, around and beside marks: the canonical text of
//! every parsed document reads back as the same document (the
//! `format_roundtrip` fuzz property), and literal delimiter characters stay
//! text.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use vellum_format::ast::{Block, Inline};
use vellum_format::{Document, parse, serialize};

type Mark = (&'static str, fn(Vec<Inline>) -> Inline);

const MARKS: &[Mark] = &[
    ("emphasis", |content| Inline::Emphasis { content }),
    ("strong", |content| Inline::Strong { content }),
    ("strikethrough", |content| Inline::Strikethrough { content }),
    ("highlight", |content| Inline::Highlight { content }),
    ("subscript", |content| Inline::Subscript { content }),
    ("superscript", |content| Inline::Superscript { content }),
    ("link", |content| Inline::Link {
        url: "https://example.com/".into(),
        title: String::new(),
        content,
    }),
];

/// Text made of delimiter characters, alone and in runs.
const DELIMITERS: &[&str] = &[
    "*", "**", "_", "__", "~", "~~", "~~~", "=", "==", "===", "^", "^^", "`", "[", "]",
];

/// What sits before and after the content under test: nothing, spaces,
/// word characters (so the marks are inside a word).
const CONTEXTS: &[(&str, &str)] = &[("", ""), ("a ", " b"), ("a", "b")];

fn text(t: &str) -> Inline {
    Inline::Text { text: t.into() }
}

fn paragraph(content: Vec<Inline>) -> Document {
    Document {
        front_matter: None,
        blocks: vec![Block::Paragraph { content, id: None }],
    }
}

/// `content` between `before` and `after`, with adjacent text merged as
/// the parser merges it.
fn in_context(content: &[Inline], (before, after): (&str, &str)) -> Document {
    let mut all: Vec<Inline> = Vec::new();
    for inline in std::iter::once(text(before))
        .chain(content.iter().cloned())
        .chain(std::iter::once(text(after)))
    {
        match (all.last_mut(), inline) {
            (_, Inline::Text { text }) if text.is_empty() => {}
            (Some(Inline::Text { text: last }), Inline::Text { text }) => last.push_str(&text),
            (_, inline) => all.push(inline),
        }
    }
    paragraph(all)
}

/// Asserts the fuzz property for `src`: what it parses to is written as
/// text that reads back the same, and that text is a fixed point.
fn assert_fixed_point(src: &str) -> Document {
    let parsed = parse(src).document;
    let text = serialize(&parsed);
    let again = parse(&text).document;
    assert_eq!(again, parsed, "{src:?} was written as {text:?}");
    assert_eq!(serialize(&again), text, "{src:?}: not idempotent");
    parsed
}

/// Writes `doc` and asserts that what it reads back as is a fixed point.
/// Returns the document read back.
fn assert_written_fixed_point(doc: &Document) -> Document {
    assert_fixed_point(&serialize(doc))
}

#[test]
fn fuzz_reproducers_round_trip() {
    // Superscripts made from long `^` runs came back as one superscript
    // holding `^^`: two adjacent superscripts wrote `^a^^b^`.
    let carets = format!(
        "Befor{}{}{}\u{e}{}{}e the break.\n",
        "^".repeat(9),
        "\0".repeat(29),
        "^".repeat(50),
        "\0".repeat(7),
        "^".repeat(50),
    );
    for src in [
        carets.as_str(),
        // `~` and `=` runs are passed over when the parser decides whether
        // a delimiter beside them opens or closes; a backslash is not.
        "p*~~~~x~~~~*q\n",
        "p**~~~x~~~**q\n",
        "p~~==~~x~~==~~q\n",
        "p==~==x==~==q\n",
        "p^~~~x~~~^q\n",
        "p~====x====~q\n",
        "p*~x*\n",
        "*x~*p\n",
        "^p~^p\n",
        "==\\~~~==\n",
        "a ==~~~== b\n",
    ] {
        let _ = assert_fixed_point(src);
    }
}

#[test]
fn adjacent_marks_of_a_kind_are_one_mark() {
    for (name, mark) in &MARKS[..6] {
        let doc = paragraph(vec![mark(vec![text("x")]), mark(vec![text("y")])]);
        let read = assert_written_fixed_point(&doc);
        assert_eq!(
            read,
            paragraph(vec![mark(vec![text("xy")])]),
            "{name}: {:?}",
            serialize(&doc)
        );
    }
}

#[test]
fn delimiter_text_inside_every_mark_round_trips() {
    for (name, mark) in MARKS {
        for delimiter in DELIMITERS {
            for content in [
                vec![text(delimiter)],
                vec![text(&format!("{delimiter}x"))],
                vec![text(&format!("x{delimiter}"))],
                vec![text(&format!("x{delimiter}y"))],
            ] {
                for &context in CONTEXTS {
                    let doc = in_context(&[mark(content.clone())], context);
                    let read = assert_written_fixed_point(&doc);
                    // Between spaces every mark can hold any text.
                    if context == CONTEXTS[1] {
                        assert_eq!(read, doc, "{name} holding {content:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn delimiter_text_beside_every_mark_round_trips() {
    for (name, mark) in MARKS {
        for delimiter in DELIMITERS {
            for content in [
                vec![text(&format!("x{delimiter}")), mark(vec![text("y")])],
                vec![mark(vec![text("x")]), text(&format!("{delimiter}y"))],
                vec![
                    text(&format!("x {delimiter}")),
                    mark(vec![text("y")]),
                    text(&format!("{delimiter} z")),
                ],
            ] {
                for &context in CONTEXTS {
                    let doc = in_context(&content, context);
                    let read = assert_written_fixed_point(&doc);
                    if context == CONTEXTS[1] {
                        assert_eq!(read, doc, "{name} beside {delimiter:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn marks_beside_and_inside_marks_round_trip() {
    let x = || vec![text("x")];
    for (_, a) in MARKS {
        for (_, b) in MARKS {
            for content in [
                vec![a(x()), b(vec![text("y")])],
                vec![a(vec![b(x())])],
                vec![a(vec![b(x()), text("y")])],
                vec![a(vec![text("y"), b(x())])],
            ] {
                for &context in CONTEXTS {
                    let _ = assert_written_fixed_point(&in_context(&content, context));
                }
            }
            for (_, c) in MARKS {
                for content in [
                    vec![a(vec![b(vec![c(x())])])],
                    vec![a(x()), b(vec![c(vec![text("y")])])],
                    vec![a(vec![b(x())]), c(vec![text("y")])],
                ] {
                    for &context in CONTEXTS {
                        let _ = assert_written_fixed_point(&in_context(&content, context));
                    }
                }
            }
        }
    }
}

#[test]
fn delimiter_sources_round_trip() {
    // Every short source built from delimiter runs and word characters.
    let tokens = [
        "a", " ", "*", "**", "~", "~~", "~~~", "=", "==", "^", "^^", "\\~",
    ];
    let mut src = String::new();
    for &t1 in &tokens {
        for &t2 in &tokens {
            for &t3 in &tokens {
                for &t4 in &tokens {
                    src.clear();
                    for t in [t1, t2, t3, t4] {
                        src.push_str(t);
                    }
                    let _ = assert_fixed_point(&src);
                }
            }
        }
    }
}
