// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for the quick-add parser (R-MW-3).

use jiff::civil::{DateTime, date};
use proptest::prelude::*;
use vellum_tasks::model::Due;
use vellum_tasks::quickadd::{TokenKind, parse, title};

fn now_strategy() -> impl Strategy<Value = DateTime> {
    (2000_i16..2100, 1_i8..=12, 1_i8..=28, 0_i8..24, 0_i8..60)
        .prop_map(|(y, m, d, h, min)| date(y, m, d).at(h, min, 0, 0))
}

/// Words that are never parsed as anything.
const PLAIN: &[&str] = &[
    "call",
    "ana",
    "about",
    "the",
    "invoice",
    "review",
    "northstar",
    "deck",
    "buy",
    "milk",
    "draft",
    "plan",
    "fix",
    "bug",
    "send",
    "notes",
    "book",
    "flights",
    "chapter",
];

/// Phrases that set a deadline day or time in the future.
const DATED: &[&str] = &[
    "today",
    "tomorrow",
    "tmr",
    "monday",
    "next tue",
    "on fri",
    "by sat",
    "nov 3",
    "3rd march",
    "in 2 weeks",
    "in 3 days",
    "in 5 hours",
    "next week",
    "next month",
    "next year",
    "end of month",
    "this weekend",
    "3pm",
    "9:15 am",
    "at 17:45",
    "noon",
    "tomorrow 3pm",
    "friday at 9am",
];

proptest! {
    #[test]
    fn never_panics_and_tokens_are_well_formed(
        input in "\\PC{0,80}",
        now in now_strategy()
    ) {
        let q = parse(&input, now);
        let mut last_end = 0;
        for token in &q.tokens {
            prop_assert!(token.span.start >= last_end);
            prop_assert!(token.span.start < token.span.end);
            prop_assert!(input.get(token.span.clone()).is_some());
            last_end = token.span.end;
        }
        prop_assert_eq!(&q.title, &title(&input, &q.tokens));
        prop_assert_eq!(q.title.trim(), q.title.as_str());
    }

    #[test]
    fn plain_words_are_kept_as_the_title(
        words in prop::collection::vec(prop::sample::select(PLAIN), 1..8),
        now in now_strategy(),
    ) {
        let input = words.join(" ");
        let q = parse(&input, now);
        prop_assert!(q.tokens.is_empty());
        prop_assert_eq!(q.title, input);
    }

    #[test]
    fn a_dated_phrase_is_highlighted_removed_and_never_in_the_past(
        before in prop::collection::vec(prop::sample::select(PLAIN), 1..4),
        phrase in prop::sample::select(DATED),
        after in prop::collection::vec(prop::sample::select(PLAIN), 0..3),
        now in now_strategy(),
    ) {
        let input = format!(
            "{} {} {}",
            before.join(" "),
            phrase,
            after.join(" ")
        );
        let q = parse(&input, now);
        let due = q.fields.due.expect("a deadline");
        match due {
            Due::Date(day) => prop_assert!(day >= now.date()),
            Due::DateTime(
                moment
            ) => prop_assert!(moment >= now || moment.date() >= now.date()),
        }
        prop_assert_eq!(q.tokens.len(), 1);
        prop_assert_eq!(q.tokens[0].kind, TokenKind::Due);
        prop_assert_eq!(input.get(q.tokens[0].span.clone()), Some(phrase));
        let expected_title = format!(
            "{} {}",
            before.join(" "),
            after.join(" ")
        );
        prop_assert_eq!(q.title, expected_title.trim());
    }

    #[test]
    fn repeat_phrases_start_on_or_after_today(
        phrase in prop::sample::select(&[
            "every day", "every weekday", "every 2 weeks", "every! 3 days",
                "every mon, thu",
            "every 15th", "every last fri", "every jan 1", "monthly",
                "every other month",
        ][..]),
        now in now_strategy(),
    ) {
        let q = parse(&format!("Water plants {phrase}"), now);
        prop_assert!(q.fields.recurrence.is_some());
        let day = q.fields.due.expect("first occurrence").date();
        prop_assert!(day >= now.date());
        prop_assert_eq!(q.title, "Water plants");
    }

    #[test]
    fn sigils_never_swallow_text(
        list in "[a-z]{1,8}", label in "[a-z]{1,8}", person in "[a-z]{1,8}",
            level in 1_u8..=4,
        now in now_strategy(),
    ) {
        let input = format!("Pack bags #{list} @{label} +{person} p{level}");
        let q = parse(&input, now);
        prop_assert_eq!(q.title, "Pack bags");
        prop_assert_eq!(q.list, Some(list));
        prop_assert_eq!(q.fields.labels, vec![label]);
        prop_assert_eq!(q.assignees, vec![person]);
        prop_assert_eq!(
            q.fields.priority.map(vellum_tasks::model::Priority::level),
            Some(level)
        );
    }
}
