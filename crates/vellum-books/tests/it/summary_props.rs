// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: the canonical SUMMARY.md always parses back to the same
//! tree, and numbering never skips or repeats.

use proptest::prelude::*;
use vellum_books::path::ChapterPath;
use vellum_books::summary::{Chapter, Item, Summary};

fn title() -> impl Strategy<Value = String> {
    "[A-Za-z0-9 ·§*_`\\[\\]<>&!#~|$()-]{1,24}".prop_filter("non-blank", |t| !t.trim().is_empty())
}

fn path() -> impl Strategy<Value = Option<ChapterPath>> {
    prop_oneof![
        1 => Just(None),
        4 => "[a-z]{1,8}(/[a-z]{1,8}){0,2}( [a-z]{1,4})?".prop_map(|p| ChapterPath::parse(&format!("{p}.md")).ok()),
    ]
}

fn chapter(depth: u32) -> BoxedStrategy<Chapter> {
    let leaf = (title(), path()).prop_map(|(name, location)| Chapter {
        name: name.split_whitespace().collect::<Vec<_>>().join(" "),
        location,
        children: Vec::new(),
    });
    if depth == 0 {
        return leaf.boxed();
    }
    (leaf, prop::collection::vec(chapter(depth - 1), 0..3))
        .prop_map(|(mut c, children)| {
            c.children = children;
            c
        })
        .boxed()
}

fn summary() -> impl Strategy<Value = Summary> {
    let flat = || {
        prop::collection::vec(
            prop_oneof![
                4 => chapter(0).prop_map(Item::Chapter),
                1 => Just(Item::Separator),
            ],
            0..4,
        )
    };
    let numbered = prop::collection::vec(
        prop_oneof![
            4 => chapter(2).prop_map(Item::Chapter),
            1 => Just(Item::Separator),
            1 => title().prop_map(|t| Item::Part(t.split_whitespace().collect::<Vec<_>>().join(" "))),
        ],
        0..5,
    );
    (proptest::option::of(title()), flat(), numbered, flat()).prop_map(
        |(title, prefix, numbered, suffix)| Summary {
            title: title.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")),
            prefix,
            numbered,
            suffix,
        },
    )
}

/// Collapses the structural ambiguities the mdBook grammar has, so the
/// generated value is what a parser can recover: a title needs something
/// before any other heading, a separator ending the numbered region before
/// an empty suffix is indistinguishable, and so on.
fn normalise(summary: &Summary) -> Option<Summary> {
    // A leading part with no title would be read as the title.
    if summary.title.is_none()
        && summary.prefix.is_empty()
        && matches!(summary.numbered.first(), Some(Item::Part(_)))
    {
        return None;
    }
    // Suffix chapters need numbered ones (or a part) before them to be suffix.
    let has_numbered = summary
        .numbered
        .iter()
        .any(|i| !matches!(i, Item::Separator));
    if !has_numbered && !summary.suffix.is_empty() {
        return None;
    }
    // A separator right after the numbered region belongs to it, as in mdBook.
    if summary
        .suffix
        .first()
        .is_some_and(|i| matches!(i, Item::Separator))
    {
        return None;
    }
    // Separators before anything numbered belong to the prefix.
    if !summary.numbered.is_empty() && matches!(summary.numbered.first(), Some(Item::Separator)) {
        return None;
    }
    if summary
        .numbered
        .iter()
        .all(|i| matches!(i, Item::Separator))
        && !summary.numbered.is_empty()
    {
        return None;
    }
    // Duplicate paths are allowed by the grammar but not by books; fine here.
    Some(summary.clone())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn canonical_summary_round_trips(summary in summary()) {
        let normalised = normalise(&summary);
        prop_assume!(normalised.is_some());
        let Some(expected) = normalised else { return Ok(()); };
        let text = expected.to_markdown();
        let parsed = Summary::parse(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
        prop_assert_eq!(&parsed, &expected, "{}", text);
        prop_assert_eq!(parsed.to_markdown(), text);
    }
}
