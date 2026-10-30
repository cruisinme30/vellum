// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Finding and rewriting links in Vellum Markdown.
//!
//! Three link forms point at other documents: ID links
//! (`vellum://doc/<id>`, the stored form), wikilinks (`[[Title]]`,
//! `[[Title|label]]`, `[[Title#heading]]`) and relative paths
//! (`[label](../Specs/Plan.md)`). ID links survive any rename or move;
//! the other two are rewritten when their target changes (R-DR-5). Links in
//! front matter, fenced code and inline code are left alone.

use std::collections::HashMap;
use std::ops::Range;

use vellum_kernel::ObjectId;
use vellum_store::BlobPath;

/// The scheme prefix of an ID link.
pub const ID_LINK_PREFIX: &str = "vellum://doc/";

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An ID link.
    Id(ObjectId),
    /// A wikilink title.
    Wiki(String),
    /// A relative path, percent-decoded, without its `#fragment`.
    Relative(String),
}

/// A link found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// What it points at.
    pub target: Target,
    /// The byte range to replace when rewriting: the title of a wikilink,
    /// or the path (without fragment) of a relative link.
    pub span: Range<usize>,
}

/// Every document link in `text`, in order.
#[must_use]
pub fn scan(text: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut in_front_matter =
        text.starts_with("---\n") || text.starts_with("---\r\n");
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += line.len();
        let content = line.trim_end_matches(['\r', '\n']);
        if in_front_matter {
            if n > 0 && (content == "---" || content == "...") {
                in_front_matter = false;
            }
            continue;
        }
        let indent = content.len() - content.trim_start_matches(' ').len();
        let trimmed = content.trim_start_matches(' ');
        if indent <= 3
            && let Some((marker, len)) = fence_marker(trimmed)
        {
            match fence {
                None => fence = Some((marker, len)),
                Some((open, open_len))
                    if open == marker
                        && len >= open_len
                        && trimmed
                            .trim_start_matches(marker)
                            .trim()
                            .is_empty() =>
                {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_none() {
            scan_line(content, start, &mut links);
        }
    }
    links
}

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = line.chars().take_while(|c| *c == marker).count();
    (len >= 3).then_some((marker, len))
}

fn scan_line(line: &str, base: usize, links: &mut Vec<Link>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes.get(i) {
            Some(b'`') => {
                let run = bytes.get(i..).map_or(0, |r| {
                    r.iter().take_while(|b| **b == b'`').count()
                });
                let fence = "`".repeat(run);
                let after = i + run;
                i = line
                    .get(after..)
                    .and_then(|rest| rest.find(&fence))
                    .map_or(after, |end| after + end + run);
            }
            Some(b'[') if bytes.get(i + 1) == Some(&b'[') => {
                let open = i + 2;
                let Some(close) = line.get(open..).and_then(|r| r.find("]]"))
                else {
                    i += 2;
                    continue;
                };
                let inner = line.get(open..open + close).unwrap_or_default();
                let title_len = inner.find(['|', '#']).unwrap_or(inner.len());
                let title = inner.get(..title_len).unwrap_or_default();
                if !title.trim().is_empty() && !inner.contains('[') {
                    let lead = title.len() - title.trim_start().len();
                    let trimmed = title.trim();
                    links.push(Link {
                        target: Target::Wiki(trimmed.to_owned()),
                        span: base + open + lead
                            ..base + open + lead + trimmed.len(),
                    });
                }
                i = open + close + 2;
            }
            Some(b']') if bytes.get(i + 1) == Some(&b'(') => {
                let open = i + 2;
                if let Some((link, end)) = inline_target(line, open, base) {
                    if let Some(link) = link {
                        links.push(link);
                    }
                    i = end;
                } else {
                    i += 2;
                }
            }
            _ => i += 1,
        }
    }
}

/// Parses the destination of an inline link starting at `open` (just
/// after `](`), returning the link (if it is a document link) and the
/// index after the closing parenthesis.
fn inline_target(
    line: &str,
    open: usize,
    base: usize,
) -> Option<(Option<Link>, usize)> {
    let rest = line.get(open..)?;
    let (dest_start, dest) = if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        (open + 1, inner.get(..end)?)
    } else {
        let mut depth = 0_i32;
        let end = rest
            .char_indices()
            .find(|(_, c)| match c {
                '(' => {
                    depth += 1;
                    false
                }
                ')' if depth == 0 => true,
                ')' => {
                    depth -= 1;
                    false
                }
                c => c.is_whitespace(),
            })
            .map_or(rest.len(), |(at, _)| at);
        (open, rest.get(..end)?)
    };
    let close = line.get(dest_start + dest.len()..)?.find(')')?
        + dest_start
        + dest.len()
        + 1;
    let path_len = dest.find(['#', '?']).unwrap_or(dest.len());
    let path = dest.get(..path_len)?;
    let span = base + dest_start..base + dest_start + path_len;
    if let Some(id) = path.strip_prefix(ID_LINK_PREFIX) {
        let found = id.parse::<ObjectId>().ok().map(|id| Link {
            target: Target::Id(id),
            span,
        });
        return Some((found, close));
    }
    let external = path.is_empty()
        || path.contains("://")
        || path.starts_with(['/', '#'])
        || has_scheme(path);
    if external {
        return Some((None, close));
    }
    let found = Link {
        target: Target::Relative(percent_decode(path)),
        span,
    };
    Some((Some(found), close))
}

fn has_scheme(path: &str) -> bool {
    path.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
            })
    })
}

/// Decodes `%XX` escapes, leaving malformed ones as they are.
#[must_use]
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// Escapes the characters that would end or break a link destination.
#[must_use]
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// Resolves a relative link from a document in `dir` (`None`: store root).
#[must_use]
pub fn resolve(dir: Option<&BlobPath>, relative: &str) -> Option<BlobPath> {
    let mut parts: Vec<&str> =
        dir.map(|d| d.segments().collect()).unwrap_or_default();
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                let _ = parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    BlobPath::new(parts.join("/")).ok()
}

/// The relative path from a document in `dir` to `to`.
#[must_use]
pub fn relative(dir: Option<&BlobPath>, to: &BlobPath) -> String {
    let from: Vec<&str> =
        dir.map(|d| d.segments().collect()).unwrap_or_default();
    let target: Vec<&str> = to.segments().collect();
    let common = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let ups = from.len() - common;
    let mut parts = vec![".."; ups];
    parts.extend(target.iter().skip(common));
    parts.join("/")
}

/// Applies replacements (byte span, new text) to `text`. Spans must not
/// overlap.
#[must_use]
pub fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(span, _)| span.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (span, new) in edits {
        if span.start < at {
            continue;
        }
        out.push_str(text.get(at..span.start).unwrap_or_default());
        out.push_str(&new);
        at = span.end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    out
}

/// Replaces every whole-token occurrence of a mapped ULID in `text`.
/// Copies use this to give every copied node a fresh ID while keeping
/// links inside the copy pointing inside the copy (R-DR-3).
#[must_use]
pub fn replace_ids<S: std::hash::BuildHasher>(
    text: &str,
    map: &HashMap<String, String, S>,
) -> String {
    if map.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        match map.get(token.as_str()) {
            Some(new) => out.push_str(new),
            None => out.push_str(token),
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> BlobPath {
        BlobPath::new(s).unwrap_or_else(|e| unreachable!("{e}"))
    }

    #[test]
    fn finds_all_three_forms() {
        let id = ObjectId::generate();
        let text = format!(
            "---\ntitle: \"[[Not]]\"\n---\n\
             See [[Plan]] and [[ Spec | the spec ]], [x](vellum://doc/{id}#h), \
             [y](../Specs/My%20Doc.md#top) [z](<a b.md>), \
             [w](https://e.com/x.md) `[[code]]`\n```\n[[fenced]]\n```\n"
        );
        let found: Vec<Target> =
            scan(&text).into_iter().map(|l| l.target).collect();
        assert_eq!(
            found,
            vec![
                Target::Wiki("Plan".into()),
                Target::Wiki("Spec".into()),
                Target::Id(id),
                Target::Relative("../Specs/My Doc.md".into()),
                Target::Relative("a b.md".into()),
            ]
        );
    }

    #[test]
    fn spans_cover_the_rewritable_part() {
        let text = "x [[Plan#h|label]] [a](dir/Plan.md#frag)";
        let links = scan(text);
        let spans: Vec<&str> = links
            .iter()
            .map(|l| text.get(l.span.clone()).unwrap_or_default())
            .collect();
        assert_eq!(spans, vec!["Plan", "dir/Plan.md"]);
        let out = apply(
            text,
            links
                .into_iter()
                .map(|l| (l.span, "New".to_owned()))
                .collect(),
        );
        assert_eq!(out, "x [[New#h|label]] [a](New#frag)");
    }

    #[test]
    fn relative_paths_resolve_and_invert() {
        let dir = p("drives/shared/eng/Specs");
        let to = p("drives/shared/eng/Plans/Q1.md");
        let rel = relative(Some(&dir), &to);
        assert_eq!(rel, "../Plans/Q1.md");
        assert_eq!(resolve(Some(&dir), &rel), Some(to));
        assert_eq!(resolve(None, "../x.md"), None);
    }

    #[test]
    fn percent_coding_round_trips() {
        for s in ["My Doc (v2).md", "50%.md", "a#b?.md"] {
            assert_eq!(percent_decode(&percent_encode(s)), s);
        }
    }

    #[test]
    fn replaces_whole_ids_only() {
        let map = HashMap::from([("AAA".to_owned(), "BBB".to_owned())]);
        assert_eq!(
            replace_ids("id: AAA\nvellum://doc/AAA AAAA", &map),
            "id: BBB\nvellum://doc/BBB AAAA"
        );
    }
}
