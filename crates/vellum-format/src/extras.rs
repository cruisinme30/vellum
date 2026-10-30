// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum inline extras found in plain text after `CommonMark` parsing:
//! `@mentions`, dates, issue keys and trailing `^blk-xxxxxxxx` block IDs.
//!
//! The rules depend only on the text itself, so the serializer can write
//! these nodes back as plain text and get the same tree when it is read.

use vellum_kernel::BlockId;

use crate::ast::Inline;

/// Merges adjacent text runs, drops empty ones and flattens a mark nested
/// directly in the same mark (`<em><em>x</em></em>` reads as `<em>x</em>`),
/// recursively.
pub fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        let inline = flatten_same(map_children(inline, merge_text));
        match (out.last_mut(), inline) {
            (_, Inline::Text { text }) if text.is_empty() => {}
            (Some(Inline::Text { text: last }), Inline::Text { text }) => {
                last.push_str(&text)
            }
            // `<em>a</em><em>b</em>` reads as `<em>ab</em>`, and the
            // delimiters of two adjacent marks of a kind would run together
            // when written (`^a^^b^` is one superscript).
            (
                Some(Inline::Emphasis { content: last }),
                Inline::Emphasis { content },
            )
            | (
                Some(Inline::Strong { content: last }),
                Inline::Strong { content },
            )
            | (
                Some(Inline::Strikethrough { content: last }),
                Inline::Strikethrough { content },
            )
            | (
                Some(Inline::Highlight { content: last }),
                Inline::Highlight { content },
            )
            | (
                Some(Inline::Subscript { content: last }),
                Inline::Subscript { content },
            )
            | (
                Some(Inline::Superscript { content: last }),
                Inline::Superscript { content },
            ) => {
                last.extend(content);
                *last = merge_text(std::mem::take(last));
            }
            (_, inline) => out.push(inline),
        }
    }
    out
}

fn flatten_same(inline: Inline) -> Inline {
    fn splice(
        content: Vec<Inline>,
        same: fn(&Inline) -> Option<&Vec<Inline>>,
    ) -> Vec<Inline> {
        let mut out = Vec::with_capacity(content.len());
        for child in content {
            match same(&child) {
                Some(inner) => out.extend(inner.iter().cloned()),
                None => out.push(child),
            }
        }
        merge_text(out)
    }
    match inline {
        Inline::Emphasis { content } => Inline::Emphasis {
            content: splice(content, |i| {
                if let Inline::Emphasis { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        Inline::Strong { content } => Inline::Strong {
            content: splice(content, |i| {
                if let Inline::Strong { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        Inline::Strikethrough { content } => Inline::Strikethrough {
            content: splice(content, |i| {
                if let Inline::Strikethrough { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        Inline::Highlight { content } => Inline::Highlight {
            content: splice(content, |i| {
                if let Inline::Highlight { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        Inline::Subscript { content } => Inline::Subscript {
            content: splice(content, |i| {
                if let Inline::Subscript { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        Inline::Superscript { content } => Inline::Superscript {
            content: splice(content, |i| {
                if let Inline::Superscript { content } = i {
                    Some(content)
                } else {
                    None
                }
            }),
        },
        other => other,
    }
}

/// Marks a backslash-escaped character that could start or join an extra
/// (`\@`, `\-`, `\^`), so [`recognize`] and [`take_block_id`] pass over it.
/// [`recognize`] removes the marks. A Unicode noncharacter is used because
/// it never appears in interchanged text.
pub const ESCAPED: char = '\u{FDD0}';

/// The text for an escaped character, marked when it matters to extras.
pub fn mark_escape(text: &str) -> String {
    if matches!(text, "@" | "-" | "^") {
        format!("{ESCAPED}{text}")
    } else {
        text.to_owned()
    }
}

/// Char offsets in `text` to backslash-escape so that no part of it is read
/// back as an extra: the `@` of a mention, the first `-` of a date or issue
/// key.
pub fn extra_escapes(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut offset = 0;
    for piece in recognize(vec![Inline::Text {
        text: text.to_owned(),
    }]) {
        let (len, escape) = match &piece {
            Inline::Text { text } => (text.chars().count(), None),
            Inline::Mention { handle } => (1 + handle.chars().count(), Some(0)),
            Inline::Date { date } if date == "today" => (6, Some(0)),
            Inline::Date { date } => (date.chars().count(), Some(4)),
            Inline::IssueKey { key } => (key.chars().count(), key.find('-')),
            _ => (0, None),
        };
        if let Some(escape) = escape {
            out.push(offset + escape);
        }
        offset += len;
    }
    out
}

/// Splits a trailing `^blk-xxxxxxxx` word off a line of directive text
/// (a fence info string, a container line or an alert title).
pub fn split_block_id(text: &str) -> (&str, Option<BlockId>) {
    let text = text.trim_end();
    let (before, last) =
        text.rsplit_once(char::is_whitespace).unwrap_or(("", text));
    last.strip_prefix('^')
        .and_then(|id| id.parse().ok())
        .map_or((text, None), |id| (before.trim_end(), Some(id)))
}

/// Removes a trailing ` ^blk-xxxxxxxx` from merged inline content.
///
/// The marker must follow whitespace (or a line break) and be the last thing
/// in the block. Returns the ID when one was found.
pub fn take_block_id(content: &mut Vec<Inline>) -> Option<BlockId> {
    let Some(Inline::Text { text }) = content.last() else {
        return None;
    };
    let start = text.rfind('^')?;
    let id: BlockId = text.get(start + 1..)?.parse().ok()?;
    let before = text.get(..start)?;
    // On its own line, or the whole paragraph (as in `- [ ] ^blk-…`).
    let own_line = before.is_empty()
        && (content.len() == 1
            || matches!(content.iter().rev().nth(1), Some(Inline::SoftBreak)));
    if !own_line && !before.ends_with(char::is_whitespace) {
        return None;
    }
    let before = before.trim_end().to_owned();
    let _ = content.pop();
    if own_line {
        let _ = content.pop();
    } else if !before.is_empty() {
        content.push(Inline::Text { text: before });
    }
    Some(id)
}

/// Splits mentions, dates and issue keys out of text runs, recursively.
/// Code spans, links, images and wikilinks are left alone.
pub fn recognize(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match inline {
            Inline::Text { text } => split_text(&text, &mut out),
            Inline::Link { .. } | Inline::Image { .. } => {
                out.push(map_children(inline, unmark))
            }
            other => out.push(map_children(other, recognize)),
        }
    }
    out
}

/// Removes escape marks without recognising extras (link text, image alt).
fn unmark(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|inline| match inline {
            Inline::Text { text } => Inline::Text {
                text: text.replace(ESCAPED, ""),
            },
            other => map_children(other, unmark),
        })
        .collect()
}

/// Applies `f` to the children of a mark, link or image.
pub fn map_children(
    inline: Inline,
    f: fn(Vec<Inline>) -> Vec<Inline>,
) -> Inline {
    match inline {
        Inline::Emphasis { content } => Inline::Emphasis {
            content: f(content),
        },
        Inline::Strong { content } => Inline::Strong {
            content: f(content),
        },
        Inline::Strikethrough { content } => Inline::Strikethrough {
            content: f(content),
        },
        Inline::Highlight { content } => Inline::Highlight {
            content: f(content),
        },
        Inline::Subscript { content } => Inline::Subscript {
            content: f(content),
        },
        Inline::Superscript { content } => Inline::Superscript {
            content: f(content),
        },
        Inline::Link {
            url,
            title,
            content,
        } => Inline::Link {
            url,
            title,
            content: f(content),
        },
        Inline::Image { url, title, alt } => Inline::Image {
            url,
            title,
            alt: f(alt),
        },
        other => other,
    }
}

fn split_text(text: &str, out: &mut Vec<Inline>) {
    let mut plain_start = 0;
    let mut index = 0;
    let mut prev: Option<char> = None;
    while let Some(c) = text.get(index..).and_then(|rest| rest.chars().next()) {
        let rest = text.get(index..).unwrap_or_default();
        let found = if at_boundary(prev) {
            match_extra(rest)
        } else {
            None
        };
        if let Some((len, node)) = found {
            if plain_start < index {
                let plain = text.get(plain_start..index).unwrap_or_default();
                out.push(Inline::Text {
                    text: plain.replace(ESCAPED, ""),
                });
            }
            out.push(node);
            index += len;
            plain_start = index;
            prev = text.get(..index).and_then(|s| s.chars().next_back());
        } else {
            index += c.len_utf8();
            prev = Some(c);
        }
    }
    if plain_start < text.len() {
        let plain = text.get(plain_start..).unwrap_or_default();
        out.push(Inline::Text {
            text: plain.replace(ESCAPED, ""),
        });
    }
}

/// An extra may start at the beginning of a run or after a character that
/// cannot be part of a word, e-mail address or longer identifier.
fn at_boundary(prev: Option<char>) -> bool {
    prev.is_none_or(|c| {
        !(c.is_alphanumeric()
            || matches!(c, '_' | '-' | '.' | '/' | '@' | ':' | ESCAPED))
    })
}

/// Matches an extra at the start of `rest`, returning its byte length.
fn match_extra(rest: &str) -> Option<(usize, Inline)> {
    let first = rest.chars().next()?;
    let (len, node) = if first == '@' {
        let handle = word(rest.get(1..)?);
        if !handle.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return None;
        }
        let node = if handle == "today" {
            Inline::Date {
                date: handle.to_owned(),
            }
        } else {
            Inline::Mention {
                handle: handle.to_owned(),
            }
        };
        (1 + handle.len(), node)
    } else if first.is_ascii_digit() {
        let date = rest.get(..10)?;
        if !is_date(date) {
            return None;
        }
        (
            10,
            Inline::Date {
                date: date.to_owned(),
            },
        )
    } else if first.is_ascii_uppercase() {
        let key = issue_key(rest)?;
        (
            key.len(),
            Inline::IssueKey {
                key: key.to_owned(),
            },
        )
    } else {
        return None;
    };
    // The extra must end at a word boundary too. A backslash does not make
    // one: the canonical text escapes `@` after a word character, so
    // `@ann\@x` must read as `@ann@x` does.
    let after = rest.get(len..)?.trim_start_matches(ESCAPED);
    let mut tail = after.chars();
    let ends_cleanly = match tail.next() {
        Some(c) if c.is_alphanumeric() || c == '_' => false,
        // `ENG-1-2`, `2026-11-24-x`, `@a-` are not extras; `@a.` is.
        Some('-' | '@') => false,
        Some('.' | ':' | '/') => {
            !tail.next().is_some_and(char::is_alphanumeric)
        }
        Some(_) | None => true,
    };
    ends_cleanly.then_some((len, node))
}

/// A handle: letters, digits and inner `.`, `_` or `-`.
fn word(s: &str) -> &str {
    let end = s
        .find(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
        .unwrap_or(s.len());
    let candidate = s.get(..end).unwrap_or_default();
    candidate.trim_end_matches(['.', '_', '-'])
}

/// `ABC-123`: 2–10 uppercase letters or digits starting with a letter, a
/// hyphen and a number without leading zeros.
fn issue_key(s: &str) -> Option<&str> {
    let dash = s.find('-')?;
    let team = s.get(..dash)?;
    let valid_team = (2..=10).contains(&team.len())
        && team
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let digits = s.get(dash + 1..)?;
    let n = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let valid_number = (1..=9).contains(&n) && !digits.starts_with('0');
    (valid_team && valid_number)
        .then(|| s.get(..=dash + n))
        .flatten()
}

/// `YYYY-MM-DD` naming a real calendar day.
pub fn is_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    let shape = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !shape {
        return false;
    }
    let num = |range: std::ops::Range<usize>| {
        s.get(range).and_then(|t| t.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day)) =
        (num(0..4), num(5..7), num(8..10))
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text { text: s.into() }
    }

    #[test]
    fn finds_mentions_dates_and_issue_keys() {
        let got = recognize(vec![text(
            "Ask @priya.s about ENG-142 by 2026-11-24, or @today.",
        )]);
        assert_eq!(
            got,
            [
                text("Ask "),
                Inline::Mention {
                    handle: "priya.s".into()
                },
                text(" about "),
                Inline::IssueKey {
                    key: "ENG-142".into()
                },
                text(" by "),
                Inline::Date {
                    date: "2026-11-24".into()
                },
                text(", or "),
                Inline::Date {
                    date: "today".into()
                },
                text("."),
            ]
        );
    }

    #[test]
    fn ignores_lookalikes() {
        for s in [
            "dev@northstar.dev",
            "ENG-0",
            "E-1",
            "eng-142",
            "ENG-142-3",
            "2026-02-30",
            "2026-11-245",
            "v2026-11-24",
            "@1st",
            "https://northstar.dev/ENG-1",
        ] {
            assert_eq!(recognize(vec![text(s)]), [text(s)], "{s}");
        }
    }

    #[test]
    fn takes_trailing_block_ids() {
        let mut content = vec![text("Ship it ^blk-a1b2c3d4")];
        assert_eq!(
            take_block_id(&mut content).map(|id| id.to_string()),
            Some("blk-a1b2c3d4".into())
        );
        assert_eq!(content, [text("Ship it")]);

        let mut content =
            vec![text("Ship it"), Inline::SoftBreak, text("^blk-a1b2c3d4")];
        assert!(take_block_id(&mut content).is_some());
        assert_eq!(content, [text("Ship it")]);

        let mut content = vec![text("x^blk-a1b2c3d4")];
        assert!(take_block_id(&mut content).is_none());
    }

    #[test]
    fn splits_block_ids_off_directive_lines() {
        let (rest, id) = split_block_id("rust title ^blk-a1b2c3d4");
        assert_eq!(rest, "rust title");
        assert_eq!(
            id.map(|id| id.to_string()).as_deref(),
            Some("blk-a1b2c3d4")
        );
        assert_eq!(split_block_id("^blk-a1b2c3d4").0, "");
        for s in ["rust", "x^blk-a1b2c3d4", "^blk-nope", "a ^blk-a1b2c3d4 b"] {
            assert_eq!(split_block_id(s), (s, None), "{s}");
        }
    }

    #[test]
    fn merges_text_runs() {
        let got = merge_text(vec![
            text("a"),
            text(""),
            text("b"),
            Inline::Strong {
                content: vec![text("c"), text("d")],
            },
        ]);
        assert_eq!(
            got,
            [
                text("ab"),
                Inline::Strong {
                    content: vec![text("cd")]
                }
            ]
        );
    }
}
