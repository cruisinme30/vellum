// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: identifiers and issue files survive a round trip.

use proptest::prelude::*;
use time::macros::datetime;
use vellum_work::{Issue, IssueId, IssueKey, Priority, StatusKey};

proptest! {
    #[test]
    fn identifiers_round_trip(
        team in "[A-Z][A-Z0-9]{1,6}",
        number in 1_u64..1_000_000_000
    ) {
        let text = format!("{team}-{number}");
        let key = IssueKey::parse(&text).unwrap();
        prop_assert_eq!(key.to_string(), text);
        prop_assert_eq!(
            IssueKey::parse(&key.to_string().to_lowercase()).unwrap(),
            key
        );
    }

    #[test]
    fn identifier_parsing_never_panics(text in "\\PC{0,20}") {
        let _ = IssueKey::parse(&text);
    }

    #[test]
    fn issue_files_round_trip(
        title in "[^\\n\\r]{1,80}",
        body in "(\\PC|\\n){0,200}",
        labels in proptest::collection::vec("[a-z]{1,10}(/[a-z]{1,10})?", 0..4),
        priority in 0_u8..=4,
    ) {
        prop_assume!(!title.trim().is_empty());
        let mut issue = Issue::new(
            IssueId::generate(),
            IssueKey::parse("ENG-7").unwrap(),
            &title,
            StatusKey::parse("todo").unwrap(),
            datetime!(2026-10-29 09:00 UTC),
        );
        issue.priority = Priority::from_number(priority).unwrap();
        issue.labels = labels;
        // A body is stored after a blank line; leading blank lines are not
        // significant in Markdown, so the writer normalises them away.
        issue.body = body.trim_start_matches('\n').to_owned();
        let text = issue.to_markdown().unwrap();
        let mut back = Issue::from_markdown(&issue.path(), &text).unwrap();
        if !issue.body.is_empty() && !issue.body.ends_with('\n') {
            let _ = back.body.pop();
        }
        prop_assert_eq!(back, issue);
    }

    #[test]
    fn scanners_never_panic(text in "(\\PC|\n|@|`|\\^){0,300}") {
        let _ = vellum_work::mention::mentions(&text);
        let _ = vellum_work::mention::issue_keys(&text);
        for block in vellum_work::mention::blocks(&text) {
            prop_assert!(text.get(block.range.clone()).is_some());
        }
    }

    #[test]
    fn chips_are_inserted_once(
        lines in proptest::collection::vec("[A-Za-z ,.]{1,40}", 1..5),
        number in 1_u64..10_000,
    ) {
        let doc = lines
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{} ^blk-{i:08}\n\n", l.trim_end()))
            .collect::<Vec<_>>()
            .concat();
        let key = IssueKey::parse(&format!("ENG-{number}")).unwrap();
        let once = vellum_work::doclink::insert_chip(
            &doc,
            "blk-00000000",
            &key
        ).unwrap();
        let twice = vellum_work::doclink::insert_chip(
            &once,
            "blk-00000000",
            &key
        ).unwrap();
        prop_assert_eq!(&once, &twice);
        prop_assert_eq!(once.len(), doc.len() + key.to_string().len() + 1);
        prop_assert_eq!(vellum_work::mention::blocks(&once).len(), lines.len());
    }
}
