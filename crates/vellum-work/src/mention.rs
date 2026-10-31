// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading Vellum Markdown for `@mentions`, issue keys and block IDs.
//!
//! Comments, issue bodies and documents all use the same inline syntax.
//! These scanners skip fenced code and inline code, so an example in a
//! code block never notifies anyone or links an issue.

use std::collections::BTreeSet;

use crate::id::UserRef;
use crate::issue::IssueKey;

/// Words written like mentions that are dates or the reader, not people.
const RESERVED: &[&str] = &["me", "today", "tomorrow", "yesterday", "now"];

/// The people mentioned in `markdown`, in order of first mention.
///
/// A mention is `@handle` at the start of a word: `name@example.com` is
/// an email, not a mention. Trailing `.`, `-` and `_` are punctuation.
/// An escaped `\@handle` is literal text, as the format parser reads it,
/// while `\\@handle` is a literal backslash followed by a mention.
#[must_use]
pub fn mentions(markdown: &str) -> Vec<UserRef> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for text in prose(markdown) {
        let bytes = text.as_bytes();
        for (i, _) in text.match_indices('@') {
            let before = i.checked_sub(1).and_then(|p| bytes.get(p).copied());
            if before.is_some_and(|b| {
                b.is_ascii_alphanumeric() || b"._-@/".contains(&b)
            }) {
                continue;
            }
            if escaped(bytes, i) {
                continue;
            }
            let rest = text.get(i + 1..).unwrap_or_default();
            let end = rest
                .find(|c: char| {
                    !(c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || "._-".contains(c))
                })
                .unwrap_or(rest.len());
            let handle = rest
                .get(..end)
                .unwrap_or_default()
                .trim_end_matches(['.', '-', '_']);
            if RESERVED.contains(&handle) {
                continue;
            }
            // An uppercase letter straight after the handle means this was
            // not a lowercase handle at all (`@Priya`).
            if rest
                .get(end..)
                .and_then(|r| r.chars().next())
                .is_some_and(|c| c.is_ascii_uppercase())
            {
                continue;
            }
            if let Ok(user) = UserRef::parse(handle)
                && seen.insert(user.clone())
            {
                out.push(user);
            }
        }
    }
    out
}

/// Whether the byte at `i` follows an odd run of backslashes.
fn escaped(bytes: &[u8], i: usize) -> bool {
    let run = bytes
        .get(..i)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|b| **b == b'\\')
        .count();
    run % 2 == 1
}

/// A block ID such as `blk-a1b2c3d4`, without the leading `^`.
#[must_use]
pub fn is_block_id(text: &str) -> bool {
    text.strip_prefix("blk-").is_some_and(|rest| {
        rest.len() == 8
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

/// One block of a document: its text and its block ID, if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'a> {
    /// The block ID from the end of its last line.
    pub id: Option<&'a str>,
    /// The block's lines, joined as written.
    pub text: &'a str,
    /// Byte range of the block in the document.
    pub range: std::ops::Range<usize>,
}

/// Splits a document body into blocks at blank lines, skipping front
/// matter and keeping fenced code as single blocks.
#[must_use]
pub fn blocks(markdown: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut fence: Option<&str> = None;
    let mut offset = 0;
    let body_start = front_matter_end(markdown);
    for line in markdown.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if line_start < body_start {
            continue;
        }
        let trimmed = line.trim();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed.get(..3).unwrap_or("```"));
            let _ = start.get_or_insert(line_start);
            continue;
        }
        if trimmed.is_empty() {
            if let Some(s) = start.take() {
                push_block(&mut out, markdown, s..line_start);
            }
        } else {
            let _ = start.get_or_insert(line_start);
        }
    }
    if let Some(s) = start {
        push_block(&mut out, markdown, s..markdown.len());
    }
    out
}

fn push_block<'a>(
    out: &mut Vec<Block<'a>>,
    markdown: &'a str,
    range: std::ops::Range<usize>,
) {
    let text = markdown.get(range.clone()).unwrap_or_default().trim_end();
    let id = text
        .rsplit_once(" ^")
        .map(|(_, id)| id)
        .or_else(|| text.strip_prefix('^'))
        .filter(|id| is_block_id(id));
    out.push(Block { id, text, range });
}

/// Where the body starts: after a leading `---` front matter block.
fn front_matter_end(markdown: &str) -> usize {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return 0;
    };
    rest.find("\n---\n")
        .map_or(0, |end| 4 + end + "\n---\n".len())
}

/// The prose of `markdown`: every line outside fenced code, with inline
/// code spans blanked out.
fn prose(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed.get(..3).unwrap_or("```"));
            continue;
        }
        out.push(blank_code_spans(line));
    }
    out
}

/// Replaces inline code (`` `like this` ``) with spaces.
fn blank_code_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Issue keys written in `text` (`ENG-142`), in order, without repeats.
///
/// A key must stand alone and be written in capitals: `xENG-1`,
/// `eng-1`, `ENG-1x` and `ENG-1-2` are not keys.
#[must_use]
pub fn issue_keys(text: &str) -> Vec<IssueKey> {
    let mut out = Vec::new();
    for line in prose(text) {
        for word in
            line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        {
            if let Ok(key) = IssueKey::parse(word)
                && key.to_string() == word
                && !out.contains(&key)
            {
                out.push(key);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handles(text: &str) -> Vec<String> {
        mentions(text).iter().map(ToString::to_string).collect()
    }

    #[test]
    fn finds_mentions_at_word_starts() {
        assert_eq!(
            handles(
                "@priya can you check with @dev.ops, and (@sam)? Thanks @priya."
            ),
            ["priya", "dev.ops", "sam"]
        );
    }

    #[test]
    fn ignores_emails_dates_code_and_capitals() {
        let text = "mail priya@northstar.dev @today `@dev`\n```\n@sam\n```\n\
            @Priya @me";
        assert!(handles(text).is_empty(), "{:?}", handles(text));
    }

    #[test]
    fn escaped_mentions_are_text() {
        assert!(handles("\\@everyone please look, cc \\@alice").is_empty());
        assert_eq!(handles("C:\\\\@priya"), ["priya"]);
    }

    #[test]
    fn finds_issue_keys_outside_code() {
        let keys: Vec<String> = issue_keys(
            "Fixes ENG-142 and OPS-7; not `ENG-9` or xENG-1 or ENG-1-2",
        )
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(keys, ["ENG-142", "OPS-7"]);
    }

    #[test]
    fn splits_blocks_with_ids() {
        let doc = "---\nid: x\n---\n# Plan ^blk-aaaaaaaa\n\nShip it\n\
            soon ^blk-bbbbbbbb\n\n```\n\n^blk-cccccccc\n```\n\nno id\n";
        let found: Vec<(Option<&str>, &str)> =
            blocks(doc).iter().map(|b| (b.id, b.text)).collect();
        assert_eq!(
            found,
            [
                (Some("blk-aaaaaaaa"), "# Plan ^blk-aaaaaaaa"),
                (Some("blk-bbbbbbbb"), "Ship it\nsoon ^blk-bbbbbbbb"),
                (None, "```\n\n^blk-cccccccc\n```"),
                (None, "no id"),
            ]
        );
    }

    #[test]
    fn block_ids_are_eight_base32_characters() {
        assert!(is_block_id("blk-a1b2c3d4"));
        assert!(!is_block_id("blk-a1b2c3d"));
        assert!(!is_block_id("blk-A1B2C3D4"));
        assert!(!is_block_id("a1b2c3d4"));
    }
}
