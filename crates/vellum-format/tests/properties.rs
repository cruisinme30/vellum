// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: parsing never panics, and the canonical serializer is a
//! fixed point that preserves the tree.

use proptest::prelude::*;
use vellum_format::{parse, serialize};

/// Checks that `src` round-trips through the canonical form.
fn assert_round_trip(src: &str) -> Result<(), TestCaseError> {
    let first = parse(src).document;
    let canonical = serialize(&first);
    let second = parse(&canonical).document;
    prop_assert_eq!(
        &second,
        &first,
        "tree changed; canonical form:\n{}",
        canonical
    );
    prop_assert_eq!(
        serialize(&second),
        canonical,
        "canonical form is not a fixed point"
    );
    Ok(())
}

/// Inline tokens of realistic prose: words, marks around words, links,
/// Vellum extras and stray punctuation.
fn token() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "Northstar",
        "launch",
        "plan",
        "the",
        "é",
        "→",
        "*it*",
        "**bold**",
        "_it_",
        "__b__",
        "~~gone~~",
        "==mark==",
        "H~2~O",
        "x^2^",
        "`code`",
        "``a`b``",
        "[docs](https://northstar.dev/docs)",
        "[t](<a b> \"T\")",
        "![chart](img/c.png)",
        "https://northstar.dev",
        "www.northstar.dev",
        "dev@northstar.dev",
        "<https://northstar.dev>",
        "@priya",
        "@today",
        "ENG-142",
        "2026-11-24",
        "[[Runbook]]",
        "[[Page|label]]",
        "^blk-a1b2c3d4",
        "5 * 3",
        "`",
        "[",
        "]",
        "<",
        ">",
        "!",
        "#",
        "|",
        "\\",
        "&",
        "&amp;",
        "&#42;",
        ":",
        "-",
        "+",
        ".",
        "1.",
        "2)",
        "$",
        "{",
        "}",
        "{#id .c k=\"v\"}",
        "\\*",
        "\\_",
        "snake_case",
        "a == b",
        "~5",
        "2^10",
    ])
}

/// Lines built from the dialect's syntax, so generated input hits the
/// interesting cases far more often than random text would.
fn line() -> impl Strategy<Value = String> {
    let prefix = prop::sample::select(vec![
        "",
        "",
        "",
        "",
        "# ",
        "## ",
        "> ",
        "- ",
        "* ",
        "1. ",
        "- [ ] ",
        "- [x] ",
        "  ",
        "    ",
        "::: info",
        "::: columns",
        ":::: tabs",
        ":::",
        "::::",
        "---",
        "***",
        "$$",
        "```",
        "```rust",
        "~~~",
        "| a | b |",
        "|---|:-:|",
        "> [!NOTE]",
        "::: details Why",
        "::: {#x}",
        "===",
        "$$ ^blk-b0000001",
        "```js ^blk-b0000002 {title=\"x\"}",
        "::: info ^blk-b0000003",
        "::: details Why ^blk-b0000004 {open}",
        "> [!TIP] Hint ^blk-b0000005",
        "- [ ] Ship {due=\"2026-11-02\"} ^blk-b0000006",
        "^blk-b0000007",
    ]);
    (prefix, prop::collection::vec(token(), 0..8)).prop_map(|(prefix, tokens)| {
        let gap = if prefix.is_empty() || prefix.ends_with(' ') {
            ""
        } else {
            " "
        };
        format!("{prefix}{gap}{}", tokens.join(" "))
    })
}

fn document() -> impl Strategy<Value = String> {
    let front = prop::sample::select(vec![
        "",
        "",
        "---\ntitle: Northstar\n---\n",
        "---\n- x\n---\n",
    ]);
    (front, prop::collection::vec(line(), 0..14))
        .prop_map(|(front, lines)| format!("{front}{}", lines.join("\n")))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn parse_never_panics(src in any::<String>()) {
        let _ = parse(&src);
    }

    #[test]
    fn dialect_documents_round_trip(src in document()) {
        assert_round_trip(&src)?;
    }
}
