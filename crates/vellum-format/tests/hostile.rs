// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red-team regression tests: hostile documents must parse on a small
//! stack, in bounded time, and still round-trip (ADR 0030, D120).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use vellum_format::{DiagnosticCode, MAX_NESTING, parse, serialize};

/// Smaller than any runtime's default thread stack (tokio and std use 2 `MiB`).
const STACK: usize = 1024 * 1024;

/// Parses, serializes and re-parses `src` on a small stack, returning the
/// diagnostics. A stack overflow aborts the test binary, so it fails loudly.
fn survive(src: String, budget: Duration) -> Vec<DiagnosticCode> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let started = Instant::now();
            let parsed = parse(&src);
            let codes: Vec<DiagnosticCode> = parsed.diagnostics.iter().map(|d| d.code).collect();
            let text = serialize(&parsed.document);
            let again = parse(&text);
            assert_eq!(
                again.document, parsed.document,
                "round trip changed the document"
            );
            assert_eq!(
                serialize(&again.document),
                text,
                "serializer is not idempotent"
            );
            let took = started.elapsed();
            assert!(took < budget, "took {took:?}, budget {budget:?}");
            codes
        })
        .unwrap()
        .join()
        .expect("parser thread panicked")
}

const DEEP: usize = 20_000;
const BUDGET: Duration = Duration::from_secs(20);

#[test]
fn deeply_nested_containers_do_not_overflow_the_stack() {
    let codes = survive("::: note\n".repeat(DEEP), BUDGET);
    assert!(codes.contains(&DiagnosticCode::NestingTooDeep));
}

#[test]
fn deeply_nested_block_quotes_do_not_overflow_the_stack() {
    let codes = survive(format!("{}deep\n", ">".repeat(DEEP)), BUDGET);
    assert!(codes.contains(&DiagnosticCode::NestingTooDeep));
}

#[test]
fn spaced_block_quotes_do_not_overflow_the_stack() {
    let _ = survive(format!("{}deep\n", "> ".repeat(DEEP)), BUDGET);
}

#[test]
fn deeply_nested_lists_do_not_overflow_the_stack() {
    let src: String = (0..400)
        .flat_map(|i| {
            [
                "  ".repeat(i),
                "- x
"
                .to_owned(),
            ]
        })
        .collect();
    let codes = survive(src, BUDGET);
    assert!(codes.contains(&DiagnosticCode::NestingTooDeep));
}

#[test]
fn quotes_inside_list_items_do_not_overflow_the_stack() {
    let _ = survive(format!("- {}x\n", ">".repeat(DEEP)), BUDGET);
}

#[test]
fn deeply_nested_emphasis_does_not_overflow_the_stack() {
    let _ = survive(format!("{}x{}", "*".repeat(DEEP), "*".repeat(DEEP)), BUDGET);
    let _ = survive(
        format!("{}x{}", "~~".repeat(DEEP), "~~".repeat(DEEP)),
        BUDGET,
    );
    let _ = survive(
        format!("{}x{}", "[".repeat(DEEP), "](u)".repeat(DEEP)),
        BUDGET,
    );
    let _ = survive(
        format!("{}x{}", "*_".repeat(DEEP / 2), "_*".repeat(DEEP / 2)),
        BUDGET,
    );
}

#[test]
fn nesting_at_the_limit_is_kept() {
    let src = format!("{}inner\n", ">".repeat(MAX_NESTING));
    let codes = survive(src, BUDGET);
    assert!(!codes.contains(&DiagnosticCode::NestingTooDeep));
}

#[test]
fn a_long_run_of_quote_markers_is_linear() {
    // comrak's alert check rescans the run at every level (was 10 s).
    let _ = survive(
        format!("{}x\n", ">".repeat(200_000)),
        Duration::from_secs(10),
    );
}

#[test]
fn quote_markers_inside_code_are_kept_byte_for_byte() {
    let run = ">".repeat(100);
    let parsed = parse(&format!("```\n{run}\n```\n"));
    assert!(serialize(&parsed.document).contains(&run));
}

#[test]
fn nested_container_output_stays_linear() {
    // Each level writes one more colon, so output grew with depth squared.
    let src = "::: note\n".repeat(5_000);
    let parsed = parse(&src);
    assert!(serialize(&parsed.document).len() < src.len() * 4);
}

#[test]
fn empty_columns_round_trip() {
    for src in [
        "::: columns\n---\n---\n:::\n",
        "::: columns\n---\n---\n---\n:::\n",
        "::: columns\n\n---\n\n:::\n",
    ] {
        let _ = survive(src.to_owned(), BUDGET);
    }
}

#[test]
fn many_open_brackets_and_backticks_are_fast() {
    for src in [
        "[".repeat(50_000),
        "`a``b".repeat(10_000),
        "![".repeat(25_000),
        "<".repeat(50_000),
        "*a".repeat(25_000),
        "&#x10FFFF;&#0;&amp;".repeat(20_000),
        "$$\n".repeat(10_000),
        "~~~\n".repeat(10_000),
        "|a".repeat(5_000) + "\n" + &"|-".repeat(5_000) + "\n",
    ] {
        let _ = survive(src, BUDGET);
    }
}

#[test]
fn hostile_front_matter_is_bounded() {
    let bomb = "---\na: &a [x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a]\n\
c: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c]\n\
e: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e]\n\
g: &g [*f,*f,*f,*f,*f,*f,*f,*f,*f]\nh: &h [*g,*g,*g,*g,*g,*g,*g,*g,*g]\n\
i: &i [*h,*h,*h,*h,*h,*h,*h,*h,*h]\n---\nbody\n";
    let _ = survive(bomb.to_owned(), BUDGET);
    let deep = format!("---\na: {}{}\n---\n", "[".repeat(DEEP), "]".repeat(DEEP));
    let _ = survive(deep, BUDGET);
    let deep_map = format!(
        "---\na: {}x{}\n---\n",
        "{b: ".repeat(DEEP),
        "}".repeat(DEEP)
    );
    let _ = survive(deep_map, BUDGET);
}

/// Fuzzer finding (`format_roundtrip`): an inline HTML tag that spans a line
/// break was kept as one text run holding `\n`, which reads back as a soft
/// break once the tag is escaped.
#[test]
fn raw_html_spanning_lines_round_trips() {
    use vellum_format::ast::{Block, Inline};
    let text = |t: &str| Inline::Text { text: t.into() };
    let doc = parse("<di\nv class=\"note\">\nNever interpreted.\n").document;
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph {
            content: vec![
                text("<di"),
                Inline::SoftBreak,
                text("v class=\"note\">"),
                Inline::SoftBreak,
                text("Never interpreted."),
            ],
            id: None,
        }]
    );
    for src in [
        "<di\nv class=\"note\">\nNever interpreted.\n",
        "<di\r\nv class=\"note\">\nNever interpreted.\n</div>\n",
        "x <di  \nv class=\"note\">\n",
        "x <di \t\n   v class=\"note\">\n",
        "x <a b='x\\\n'> y\n",
        "<a\nb>\n===\n",
        "- x <a\n  b> y\n",
    ] {
        let codes = survive(src.to_owned(), BUDGET);
        assert!(codes.contains(&DiagnosticCode::RawHtml), "{src:?}");
    }
}
