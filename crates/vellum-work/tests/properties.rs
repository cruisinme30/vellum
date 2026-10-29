// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: identifiers and issue files survive a round trip.

use proptest::prelude::*;
use time::macros::datetime;
use vellum_work::{Issue, IssueId, IssueKey, Priority, StatusKey};

proptest! {
    #[test]
    fn identifiers_round_trip(team in "[A-Z][A-Z0-9]{1,6}", number in 1_u64..1_000_000_000) {
        let text = format!("{team}-{number}");
        let key = IssueKey::parse(&text).unwrap();
        prop_assert_eq!(key.to_string(), text);
        prop_assert_eq!(IssueKey::parse(&key.to_string().to_lowercase()).unwrap(), key);
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
}
