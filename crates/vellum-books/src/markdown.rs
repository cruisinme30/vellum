// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The small amount of Markdown surgery books need: front matter fields,
//! link destinations and headings, located by exact byte ranges so edits
//! touch nothing else in the file.
//!
//! Links inside code blocks, code spans and math are never touched,
//! because the parser does not report them as links.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::id::NodeId;

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Where the YAML front matter and the body of a file are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// The YAML between the `---` fences, if the file has front matter.
    pub yaml: Option<Range<usize>>,
    /// Where the closing fence ends and the body starts.
    pub body: usize,
}

/// Locates the front matter of a Vellum Markdown file.
#[must_use]
pub fn split(text: &str) -> Split {
    let none = Split {
        yaml: None,
        body: 0,
    };
    let Some(first_end) = text.find('\n') else {
        return none;
    };
    if text.get(..first_end).map(str::trim_end) != Some("---") {
        return none;
    }
    let start = first_end + 1;
    let mut offset = start;
    for line in text.get(start..).unwrap_or_default().split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed == "---" || trimmed == "..." {
            return Split {
                yaml: Some(start..offset),
                body: offset + line.len(),
            };
        }
        offset += line.len();
    }
    none
}

/// The body of a file without its front matter, leading blank lines
/// removed.
#[must_use]
pub fn body(text: &str) -> &str {
    text.get(split(text).body..)
        .unwrap_or_default()
        .trim_start_matches(['\n', '\r'])
}

/// A top-level scalar from the front matter, as a string.
#[must_use]
pub fn front_matter(text: &str, key: &str) -> Option<String> {
    let range = split(text).yaml?;
    let yaml = text.get(range)?;
    let fields: std::collections::BTreeMap<String, serde_json::Value> =
        serde_saphyr::from_str(yaml).ok()?;
    match fields.get(key)? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn yaml_scalar(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    serde_saphyr::to_string(&flat).map_or_else(|_| format!("{flat:?}"), |s| s.trim_end().to_owned())
}

/// Sets a top-level front-matter field to a single-line string, adding
/// front matter when the file has none. Other lines are kept byte for
/// byte.
#[must_use]
pub fn set_front_matter(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}: {}\n", yaml_scalar(value));
    let parts = split(text);
    let Some(yaml) = parts.yaml else {
        return format!("---\n{line}---\n\n{text}");
    };
    let mut out = String::with_capacity(text.len() + line.len());
    out.push_str(text.get(..yaml.start).unwrap_or_default());
    let mut replaced = false;
    let mut skipping = false;
    for existing in text
        .get(yaml.clone())
        .unwrap_or_default()
        .split_inclusive('\n')
    {
        if skipping && existing.starts_with([' ', '\t']) {
            continue;
        }
        skipping = false;
        let is_key = existing
            .strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(':'));
        if is_key && !replaced {
            out.push_str(&line);
            replaced = true;
            skipping = true;
        } else {
            out.push_str(existing);
        }
    }
    if !replaced {
        out.push_str(&line);
    }
    out.push_str(text.get(yaml.end..).unwrap_or_default());
    out
}

/// The text of a new chapter file.
#[must_use]
pub fn new_chapter(id: &NodeId, title: &str, status: Option<&str>, body: &str) -> String {
    let mut text = format!("---\nid: {id}\ntitle: {}\n", yaml_scalar(title));
    if let Some(status) = status {
        text.push_str("status: ");
        text.push_str(status);
        text.push('\n');
    }
    text.push_str("vellum: 1\n---\n");
    let body = body.trim_matches(['\n', '\r']);
    if !body.is_empty() {
        text.push('\n');
        text.push_str(body);
        text.push('\n');
    }
    text
}

/// What kind of reference a [`LinkRef`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[text](dest)`.
    Inline,
    /// `<dest>`.
    Autolink,
    /// `[label]: dest`, used by reference-style links.
    Definition,
    /// `![alt](dest)`.
    Image,
}

/// A link destination found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    /// What kind of link it is.
    pub kind: LinkKind,
    /// The destination as parsed.
    pub dest: String,
    /// Where the destination is written in the file.
    pub dest_range: Range<usize>,
    /// The whole link, `[text](dest)`.
    pub link_range: Range<usize>,
    /// The link text between the brackets (inline links only).
    pub text_range: Option<Range<usize>>,
}

/// Every link, image and reference definition in a file's body whose
/// destination is written literally (so it can be rewritten in place).
#[must_use]
pub fn links(text: &str) -> Vec<LinkRef> {
    let offset = split(text).body;
    let body = text.get(offset..).unwrap_or_default();
    let mut out = Vec::new();
    let mut iter = Parser::new_ext(body, options()).into_offset_iter();
    for (event, range) in iter.by_ref() {
        let (kind, dest) = match &event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => match link_type {
                LinkType::Inline => (LinkKind::Inline, dest_url.to_string()),
                LinkType::Autolink => (LinkKind::Autolink, dest_url.to_string()),
                _ => continue,
            },
            Event::Start(Tag::Image {
                link_type: LinkType::Inline,
                dest_url,
                ..
            }) => (LinkKind::Image, dest_url.to_string()),
            _ => continue,
        };
        let Some(source) = body.get(range.clone()) else {
            continue;
        };
        if dest.is_empty() {
            // `[text]()`: the destination is the empty string before `)`.
            let Some(close) = source.rfind(')') else {
                continue;
            };
            let Some(open) = source.get(..close).and_then(|s| s.rfind('(')) else {
                continue;
            };
            out.push(LinkRef {
                kind,
                dest,
                dest_range: offset + range.start + open + 1..offset + range.start + close,
                link_range: offset + range.start..offset + range.end,
                text_range: text_range(source, open, offset + range.start, kind),
            });
            continue;
        }
        let Some(at) = source.rfind(dest.as_str()) else {
            continue;
        };
        out.push(LinkRef {
            kind,
            dest_range: offset + range.start + at..offset + range.start + at + dest.len(),
            link_range: offset + range.start..offset + range.end,
            text_range: text_range(source, at, offset + range.start, kind),
            dest,
        });
    }
    for (_, definition) in iter.reference_definitions().iter() {
        let span = definition.span.clone();
        let Some(source) = body.get(span.clone()) else {
            continue;
        };
        let Some(colon) = source.find("]:") else {
            continue;
        };
        let dest = definition.dest.to_string();
        let Some(at) = source
            .get(colon..)
            .and_then(|rest| rest.find(dest.as_str()))
            .map(|i| i + colon)
        else {
            continue;
        };
        out.push(LinkRef {
            kind: LinkKind::Definition,
            dest_range: offset + span.start + at..offset + span.start + at + dest.len(),
            link_range: offset + span.start..offset + span.end,
            text_range: None,
            dest,
        });
    }
    out.sort_by_key(|link| link.link_range.start);
    out
}

fn text_range(source: &str, dest_at: usize, base: usize, kind: LinkKind) -> Option<Range<usize>> {
    if kind != LinkKind::Inline {
        return None;
    }
    let close = source.get(..dest_at)?.rfind(']')?;
    Some(base + 1..base + close)
}

/// A heading found in a file's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingRef {
    /// 1 to 6.
    pub level: u8,
    /// The heading's lines, from the start of its first line to the end of
    /// its last line (including the newline).
    pub range: Range<usize>,
    /// The plain text of the heading.
    pub text: String,
    /// The heading's inline Markdown source, without `#` marks, closing
    /// sequence or setext underline.
    pub source: String,
    /// The anchor mdBook and the renderer give it.
    pub anchor: String,
}

/// Every heading in a file's body, in order.
#[must_use]
pub fn headings(text: &str) -> Vec<HeadingRef> {
    let offset = split(text).body;
    let body = text.get(offset..).unwrap_or_default();
    let mut out = Vec::new();
    let mut current: Option<(u8, Range<usize>, Option<String>, String)> = None;
    for (event, range) in Parser::new_ext(body, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                current = Some((
                    level as u8,
                    range,
                    id.map(|id| id.to_string()),
                    String::new(),
                ));
            }
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) => {
                if let Some((_, _, _, text)) = current.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, _, text)) = current.as_mut() {
                    text.push(' ');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                let Some((level, range, id, text)) = current.take() else {
                    continue;
                };
                let start = body
                    .get(..range.start)
                    .map_or(0, |b| b.rfind('\n').map_or(0, |i| i + 1));
                let end = body
                    .get(range.end..)
                    .and_then(|rest| {
                        if body.get(..range.end).is_some_and(|b| b.ends_with('\n')) {
                            Some(0)
                        } else {
                            rest.find('\n').map(|i| i + 1)
                        }
                    })
                    .map_or(body.len(), |extra| range.end + extra);
                let raw = body.get(start..end).unwrap_or_default();
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push(HeadingRef {
                    level,
                    range: offset + start..offset + end,
                    anchor: id.unwrap_or_else(|| anchor(&text)),
                    source: heading_source(raw),
                    text,
                });
            }
            _ => {}
        }
    }
    out
}

/// The inline source of a heading block (ATX or setext).
fn heading_source(raw: &str) -> String {
    let trimmed = raw.trim_start_matches(' ');
    if let Some(rest) = trimmed.strip_prefix('#') {
        let rest = rest.trim_start_matches('#');
        let line = rest.lines().next().unwrap_or_default().trim();
        // Drop an optional closing sequence of `#`s preceded by a space.
        let without_close = line.trim_end_matches('#');
        if without_close.len() < line.len()
            && (without_close.is_empty() || without_close.ends_with(' '))
        {
            return without_close.trim_end().to_owned();
        }
        return line.to_owned();
    }
    let lines: Vec<&str> = raw.lines().collect();
    let content = lines
        .get(..lines.len().saturating_sub(1))
        .unwrap_or_default();
    content
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The anchor of a heading, as mdBook computes it: lower case, letters,
/// digits, `_` and `-` kept, whitespace turned into `-`.
#[must_use]
pub fn anchor(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c.to_lowercase().collect::<String>())
            } else if c.is_whitespace() {
                Some("-".to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// An ATX heading line at `level` (capped at 6) with the given source.
#[must_use]
pub fn heading_line(level: usize, source: &str) -> String {
    format!("{} {}\n", "#".repeat(level.clamp(1, 6)), source)
}

/// Applies non-overlapping replacements to `text`. Overlapping edits after
/// the first are dropped.
#[must_use]
pub fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (range, replacement) in edits {
        if range.start < cursor || range.end > text.len() {
            continue;
        }
        out.push_str(text.get(cursor..range.start).unwrap_or_default());
        out.push_str(&replacement);
        cursor = range.end;
    }
    out.push_str(text.get(cursor..).unwrap_or_default());
    out
}

/// A change to one link, returned by the callback of [`map_links`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkEdit {
    /// A new destination.
    pub dest: Option<String>,
    /// New link text (plain text; it is escaped). Inline links only.
    pub text: Option<String>,
    /// Replace the whole link with this plain text (it is escaped).
    /// Inline links and autolinks only.
    pub unlink: Option<String>,
}

/// Rewrites links in a file. `edit` sees each link and its current text
/// (for inline links) and returns what to change, if anything.
#[must_use]
pub fn map_links(
    text: &str,
    mut edit: impl FnMut(&LinkRef, Option<&str>) -> Option<LinkEdit>,
) -> String {
    let mut edits = Vec::new();
    for link in links(text) {
        let current = link.text_range.clone().and_then(|r| text.get(r));
        let Some(change) = edit(&link, current) else {
            continue;
        };
        if let Some(plain) = change.unlink {
            if matches!(link.kind, LinkKind::Inline | LinkKind::Autolink) {
                edits.push((link.link_range.clone(), escape_link_text(&plain)));
            }
            continue;
        }
        if let (Some(new_text), Some(range)) = (change.text, link.text_range.clone()) {
            edits.push((range, escape_link_text(&new_text)));
        }
        if let Some(dest) = change.dest {
            edits.push((link.dest_range.clone(), dest));
        }
    }
    apply(text, edits)
}

/// Escapes text for use inside a Markdown link's brackets.
#[must_use]
pub fn escape_link_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '&' | '!' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---
id: 01JBK0000000000000000000A1
title: Services
vellum: 1
---

# Services

See [the gateway](vellum://doc/01JBK0000000000000000000A2) and
[§](vellum://doc/01JBK0000000000000000000A3), [ref][r] and <vellum://doc/01JBK0000000000000000000A4>.

```text
[not a link](vellum://doc/01JBK0000000000000000000A5)
```

![diagram](../assets/map.png)

Setext heading
--------------

## Closing ##

[r]: vellum://doc/01JBK0000000000000000000A6 \"Title\"
";

    #[test]
    fn front_matter_fields() {
        assert_eq!(front_matter(DOC, "title").as_deref(), Some("Services"));
        assert_eq!(front_matter(DOC, "vellum").as_deref(), Some("1"));
        let renamed = set_front_matter(DOC, "title", "Services: an overview");
        assert_eq!(
            front_matter(&renamed, "title").as_deref(),
            Some("Services: an overview")
        );
        assert_eq!(body(&renamed), body(DOC));
        let added = set_front_matter(DOC, "status", "draft");
        assert_eq!(front_matter(&added, "status").as_deref(), Some("draft"));
        let bare = set_front_matter("Hello\n", "id", "01JBK0000000000000000000A1");
        assert_eq!(
            front_matter(&bare, "id").as_deref(),
            Some("01JBK0000000000000000000A1")
        );
        assert_eq!(body(&bare), "Hello\n");
    }

    #[test]
    fn finds_links_outside_code() {
        let found = links(DOC);
        let dests: Vec<&str> = found.iter().map(|l| l.dest.as_str()).collect();
        assert_eq!(
            dests,
            [
                "vellum://doc/01JBK0000000000000000000A2",
                "vellum://doc/01JBK0000000000000000000A3",
                "vellum://doc/01JBK0000000000000000000A4",
                "../assets/map.png",
                "vellum://doc/01JBK0000000000000000000A6",
            ]
        );
        for link in &found {
            assert_eq!(DOC.get(link.dest_range.clone()), Some(link.dest.as_str()));
        }
        let xref = found.get(1).and_then(|l| l.text_range.clone());
        assert_eq!(xref.and_then(|r| DOC.get(r)), Some("§"));
    }

    #[test]
    fn finds_headings() {
        let found = headings(DOC);
        let summary: Vec<(u8, &str, &str)> = found
            .iter()
            .map(|h| (h.level, h.source.as_str(), h.anchor.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (1, "Services", "services"),
                (2, "Setext heading", "setext-heading"),
                (2, "Closing", "closing"),
            ]
        );
        let first = found.first().map(|h| h.range.clone());
        assert_eq!(first.and_then(|r| DOC.get(r)), Some("# Services\n"));
    }

    #[test]
    fn applies_edits() {
        let text = "abcdef";
        assert_eq!(
            apply(text, vec![(4..5, "E".into()), (0..1, "A".into())]),
            "AbcdEf"
        );
        assert_eq!(
            apply(text, vec![(0..3, "x".into()), (2..4, "y".into())]),
            "xdef"
        );
    }
}
