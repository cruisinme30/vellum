// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Finding issue keys and magic words in branches, titles, descriptions
//! and commit messages.
//!
//! | Where | Bare key | After a closing word | After a linking word |
//! |---|---|---|---|
//! | Branch name | closing | – | – |
//! | Pull request title | closing | closing | linking |
//! | Description, commit message | ignored | closing | linking |
//!
//! Closing words: close, fix, resolve, complete in every form (`closes`,
//! `fixed`, `resolving` …). Linking words: ref, refs, references, part of,
//! related to, contributes to, towards. A word applies to the list of keys
//! after it, joined by commas, `and` or `&`. Keys may be written as issue
//! URLs. An explicit word beats an implicit one, and linking beats closing,
//! so `Part of ENG-142` in a description stops a title mention from
//! completing the issue (D79).
//!
//! Only keys of teams the integration may act on survive ("team key must
//! match"); `PR-12` or `UTF-8` never touch an issue.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::IssueKey;

/// What a reference asks for.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// Link, and drive the issue's state; merging completes it.
    Closing,
    /// Link only; the issue's state is never changed.
    Linking,
}

/// Where the text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// A branch name.
    Branch,
    /// A pull or merge request title.
    Title,
    /// A pull request description.
    Body,
    /// A commit message.
    Commit,
}

/// One issue a pull request or commit refers to.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Reference {
    /// The issue.
    pub key: IssueKey,
    /// Closing or linking.
    pub kind: RefKind,
}

const CLOSING: [&str; 16] = [
    "close",
    "closes",
    "closed",
    "closing",
    "fix",
    "fixes",
    "fixed",
    "fixing",
    "resolve",
    "resolves",
    "resolved",
    "resolving",
    "complete",
    "completes",
    "completed",
    "completing",
];

fn closing_word(word: &str) -> bool {
    CLOSING.contains(&word)
}

fn linking_word(word: &str, next: Option<&str>) -> Option<usize> {
    match (word, next) {
        ("ref" | "refs" | "references" | "towards" | "toward", _) => Some(1),
        ("part" | "related" | "contributes" | "contributing", Some(n))
            if n == "of" || n == "to" =>
        {
            Some(2)
        }
        _ => None,
    }
}

const PUNCTUATION: &[char] = &[
    '(', ')', '[', ']', '{', '}', '.', ',', ';', ':', '!', '?', '"', '\'', '`',
    '*', '<', '>',
];

/// A key written as `ENG-142`, or inside a URL path as `/issue/ENG-142`.
fn key_in_token(token: &str) -> Option<IssueKey> {
    if let Ok(key) = IssueKey::parse(token) {
        return Some(key);
    }
    if token.contains("://") {
        let mut segments = token.split('/');
        while let Some(segment) = segments.next() {
            if segment == "issue" {
                return segments.next().and_then(|s| IssueKey::parse(s).ok());
            }
        }
    }
    None
}

/// `(key, kind, explicit)` hits in free text.
fn scan_text(
    text: &str,
    implicit: Option<RefKind>,
) -> Vec<(IssueKey, RefKind, bool)> {
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(|t| t.trim_matches(PUNCTUATION).to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    let mut hits = Vec::new();
    let mut active: Option<RefKind> = None;
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        let lower = token.to_ascii_lowercase();
        if let Some(key) = key_in_token(token) {
            match active {
                Some(kind) => hits.push((key, kind, true)),
                None => {
                    if let Some(kind) = implicit {
                        hits.push((key, kind, false));
                    }
                }
            }
            i += 1;
            continue;
        }
        if closing_word(&lower) {
            active = Some(RefKind::Closing);
            i += 1;
            continue;
        }
        let next = tokens.get(i + 1).map(|t| t.to_ascii_lowercase());
        if let Some(width) = linking_word(&lower, next.as_deref()) {
            active = Some(RefKind::Linking);
            i += width;
            continue;
        }
        if !(lower == "and" || lower == "&") {
            active = None;
        }
        i += 1;
    }
    hits
}

/// Keys in a branch name such as `ada/eng-142-fix-login`.
fn scan_branch(branch: &str) -> Vec<(IssueKey, RefKind, bool)> {
    let mut hits = Vec::new();
    for segment in branch.split('/') {
        let parts: Vec<&str> = segment.split(['-', '_']).collect();
        for pair in parts.windows(2) {
            if let [team, number] = pair
                && let Ok(key) = IssueKey::parse(&format!("{team}-{number}"))
            {
                hits.push((key, RefKind::Closing, false));
            }
        }
    }
    hits
}

/// Collects references from several texts and resolves conflicts.
#[derive(Debug, Default, Clone)]
pub struct Collector {
    hits: Vec<(IssueKey, RefKind, bool)>,
}

impl Collector {
    /// An empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the references in `text` found at `location`.
    pub fn add(&mut self, location: Location, text: &str) -> &mut Self {
        let found = match location {
            Location::Branch => scan_branch(text),
            Location::Title => scan_text(text, Some(RefKind::Closing)),
            Location::Body | Location::Commit => scan_text(text, None),
        };
        self.hits.extend(found);
        self
    }

    /// The references to teams in `teams`, one per issue.
    #[must_use]
    pub fn finish(&self, teams: &BTreeSet<String>) -> Vec<Reference> {
        // (any explicit, explicit linking, implicit kinds)
        let mut merged: BTreeMap<&IssueKey, (bool, bool)> = BTreeMap::new();
        for (key, kind, explicit) in &self.hits {
            if !teams.contains(&key.team) {
                continue;
            }
            let entry = merged.entry(key).or_insert((false, false));
            if *explicit {
                entry.0 = true;
                entry.1 |= *kind == RefKind::Linking;
            }
        }
        let mut out: Vec<Reference> = merged
            .into_iter()
            .map(|(key, (explicit, explicit_linking))| {
                let implicit_kind = self
                    .hits
                    .iter()
                    .filter(|(k, _, e)| k == key && !e)
                    .map(|(_, kind, _)| *kind)
                    .max()
                    .unwrap_or(RefKind::Closing);
                let kind = if explicit_linking
                    || (!explicit && implicit_kind == RefKind::Linking)
                {
                    RefKind::Linking
                } else {
                    RefKind::Closing
                };
                Reference {
                    key: key.clone(),
                    kind,
                }
            })
            .collect();
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn teams() -> BTreeSet<String> {
        ["ENG", "OPS"].iter().map(|t| (*t).to_owned()).collect()
    }

    fn refs(items: &[(Location, &str)]) -> Vec<String> {
        let mut collector = Collector::new();
        for (location, text) in items {
            let _ = collector.add(*location, text);
        }
        collector
            .finish(&teams())
            .into_iter()
            .map(|r| format!("{}:{:?}", r.key, r.kind))
            .collect()
    }

    #[test]
    fn branch_names() {
        assert_eq!(
            refs(&[(Location::Branch, "ada/eng-142-fix-login")]),
            ["ENG-142:Closing"]
        );
        assert_eq!(
            refs(&[(Location::Branch, "feature/ENG_7_ops-9")]),
            ["ENG-7:Closing", "OPS-9:Closing"]
        );
        assert!(refs(&[(Location::Branch, "fix/utf-8-handling")]).is_empty());
    }

    #[test]
    fn closing_word_forms_and_lists() {
        assert_eq!(
            refs(&[(
                Location::Body,
                "This fixes ENG-1, ENG-2 and ENG-3. Also mentions OPS-4."
            )]),
            ["ENG-1:Closing", "ENG-2:Closing", "ENG-3:Closing"]
        );
        for word in [
            "Closes",
            "closed",
            "closing",
            "Resolves:",
            "resolved",
            "completes",
            "Fixed",
            "fixing",
        ] {
            assert_eq!(
                refs(&[(Location::Commit, &format!("{word} ENG-9"))]),
                ["ENG-9:Closing"],
                "{word}"
            );
        }
        assert!(refs(&[(Location::Body, "fixture ENG-9")]).is_empty());
    }

    #[test]
    fn linking_words() {
        assert_eq!(
            refs(&[(Location::Body, "Part of ENG-5 & ENG-6")]),
            ["ENG-5:Linking", "ENG-6:Linking"]
        );
        assert_eq!(
            refs(&[(Location::Body, "contributes to OPS-1")]),
            ["OPS-1:Linking"]
        );
        assert_eq!(
            refs(&[(Location::Body, "refs ENG-2; related to ENG-3")]),
            ["ENG-2:Linking", "ENG-3:Linking"]
        );
        assert_eq!(
            refs(&[(Location::Body, "towards ENG-4")]),
            ["ENG-4:Linking"]
        );
    }

    #[test]
    fn explicit_linking_beats_implicit_closing() {
        assert_eq!(
            refs(&[
                (Location::Title, "ENG-5 Add exporter"),
                (Location::Body, "Part of ENG-5")
            ]),
            ["ENG-5:Linking"]
        );
        assert_eq!(
            refs(&[
                (Location::Branch, "ada/eng-5-x"),
                (Location::Body, "Fixes ENG-5")
            ]),
            ["ENG-5:Closing"]
        );
    }

    #[test]
    fn urls_and_unknown_teams() {
        assert_eq!(
            refs(&[(
                Location::Body,
                "Closes https://vellum.northstar.dev/issue/ENG-77/fix-it"
            )]),
            ["ENG-77:Closing"]
        );
        assert!(
            refs(&[(
                Location::Title,
                "Bump UTF-8 and SHA-256 handling, fixes PR-12"
            )])
            .is_empty()
        );
    }
}
