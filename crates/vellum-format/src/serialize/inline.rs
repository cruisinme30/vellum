// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Writes inline content as canonical Markdown, escaping text so that it
//! reads back as the same inline tree.

use crate::ast::Inline;

/// Where the inline content sits; some characters only need escaping there.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A paragraph or list item.
    Paragraph,
    /// A heading line, where `#` and `{` would close it or start attributes.
    Heading,
    /// A table cell.
    Cell,
}

/// Writes `inlines` to a string.
pub fn write(inlines: &[Inline], place: Place) -> String {
    let mut w = Writer {
        out: String::new(),
        place,
        line_start: true,
        bare_ok_at_end: true,
        bracket_next: false,
    };
    w.inlines(inlines, false);
    w.out
}

struct Writer {
    out: String,
    place: Place,
    /// True at the start of the content or after a line break.
    line_start: bool,
    /// Whether a bare autolink may end the current content: false inside
    /// marks whose closing characters the autolink would swallow.
    bare_ok_at_end: bool,
    /// The next sibling starts with `[`, so a trailing `!` would make it an image.
    bracket_next: bool,
}

impl Writer {
    /// `nested_mark`: the parent is emphasis or strong, so an edge child of
    /// the same kind uses `_` delimiters to keep its nesting.
    fn inlines(&mut self, inlines: &[Inline], nested_mark: bool) {
        let last = inlines.len().saturating_sub(1);
        for (i, inline) in inlines.iter().enumerate() {
            let at_edge = nested_mark && (i == 0 || i == last);
            let bare_ok = match inlines.get(i + 1) {
                None => self.bare_ok_at_end,
                Some(Inline::SoftBreak | Inline::HardBreak) => true,
                Some(Inline::Text { text }) => text.starts_with(char::is_whitespace),
                Some(_) => false,
            };
            self.bracket_next = matches!(inlines.get(i + 1), Some(Inline::Link { .. } | Inline::WikiLink { .. }));
            self.inline(inline, at_edge, bare_ok);
        }
    }

    fn inline(&mut self, inline: &Inline, at_edge: bool, bare_ok: bool) {
        let starts_line = self.line_start;
        self.line_start = false;
        match inline {
            Inline::Text { text } => self.text(text, starts_line),
            Inline::Code { code } => self.code(code),
            Inline::SoftBreak => {
                self.out.push('\n');
                self.line_start = true;
            }
            Inline::HardBreak => {
                self.out.push_str("\\\n");
                self.line_start = true;
            }
            Inline::Emphasis { content } => {
                self.wrap(if at_edge { "_" } else { "*" }, content, true);
            }
            Inline::Strong { content } => {
                self.wrap(if at_edge { "__" } else { "**" }, content, true);
            }
            Inline::Strikethrough { content } => self.wrap("~~", content, false),
            Inline::Highlight { content } => self.wrap("==", content, false),
            Inline::Subscript { content } => self.wrap("~", content, false),
            Inline::Superscript { content } => self.wrap("^", content, false),
            Inline::Link {
                url,
                title,
                content,
            } => {
                if let Some(bare) = bare_autolink(url, title, content).filter(|_| bare_ok) {
                    self.out.push_str(bare);
                } else if let Some(auto) = autolink(url, title, content) {
                    self.out.push('<');
                    self.out.push_str(auto);
                    self.out.push('>');
                } else {
                    self.out.push('[');
                    self.inlines(content, false);
                    self.out.push(']');
                    self.destination(url, title);
                }
            }
            Inline::Image { url, title, alt } => {
                self.out.push_str("![");
                self.inlines(alt, false);
                self.out.push(']');
                self.destination(url, title);
            }
            Inline::WikiLink { target, label } => {
                self.out.push_str("[[");
                self.out.push_str(target);
                if let Some(label) = label {
                    self.out.push('|');
                    self.out.push_str(label);
                }
                self.out.push_str("]]");
            }
            Inline::Mention { handle } => {
                self.out.push('@');
                self.out.push_str(handle);
            }
            Inline::IssueKey { key } => self.out.push_str(key),
            Inline::Date { date } if date == "today" => self.out.push_str("@today"),
            Inline::Date { date } => self.out.push_str(date),
        }
    }

    fn wrap(&mut self, mark: &str, content: &[Inline], emphasis: bool) {
        self.out.push_str(mark);
        let outer = self.bare_ok_at_end;
        self.bare_ok_at_end = true;
        self.inlines(content, emphasis);
        self.bare_ok_at_end = outer;
        self.out.push_str(mark);
    }

    fn code(&mut self, code: &str) {
        let longest = longest_run(code, '`');
        let fence = "`".repeat(longest + 1);
        let pad = code.starts_with('`')
            || code.ends_with('`')
            || (code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty());
        self.out.push_str(&fence);
        if pad {
            self.out.push(' ');
        }
        self.out.push_str(code);
        if pad {
            self.out.push(' ');
        }
        self.out.push_str(&fence);
    }

    fn destination(&mut self, url: &str, title: &str) {
        self.out.push('(');
        let pointy = url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '(' | ')' | '<' | '>'));
        if pointy {
            self.out.push('<');
            push_escaped(&mut self.out, url, &['<', '>', '\\']);
            self.out.push('>');
        } else {
            push_escaped(&mut self.out, url, &['\\']);
        }
        if !title.is_empty() {
            self.out.push_str(" \"");
            push_escaped(&mut self.out, title, &['"', '\\']);
            self.out.push('"');
        }
        self.out.push(')');
    }

    fn text(&mut self, text: &str, starts_line: bool) {
        let chars: Vec<char> = text.chars().collect();
        let marker_end = if starts_line {
            line_start_escape(&chars)
        } else {
            None
        };
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
            let next = chars.get(i + 1).copied();
            let escape = match c {
                '\\' | '`' | '*' | '[' | ']' | '<' | '~' | '^' | '|' => true,
                '_' => {
                    !(prev.is_some_and(char::is_alphanumeric)
                        && next.is_some_and(char::is_alphanumeric))
                }
                '=' => prev.is_none_or(|p| p == '=') || next.is_none_or(|n| n == '='),
                '&' => next.is_some_and(|n| n == '#' || n.is_ascii_alphabetic()),
                '!' => next.is_none() && self.bracket_next,
                '#' | '{' => self.place == Place::Heading,
                // Break up what the autolink extension would turn into links.
                ':' => chars.get(i + 1..i + 3) == Some(&['/', '/']),
                '.' => i >= 3 && chars.get(i - 3..i).is_some_and(|w| w.iter().all(|c| c.eq_ignore_ascii_case(&'w'))),
                '@' => prev.is_some_and(|p| p.is_alphanumeric() || matches!(p, '.' | '_' | '+' | '-')),
                _ => false,
            } || (starts_line && Some(i) == marker_end);
            if escape {
                self.out.push('\\');
            }
            self.out.push(c);
        }
    }
}

/// Which character of a line-leading text must be escaped so it is not
/// read as a block marker (heading, quote, list, setext line, container or
/// math fence).
fn line_start_escape(chars: &[char]) -> Option<usize> {
    let first = *chars.first()?;
    match first {
        '#' | '>' | '-' | '+' | '=' => Some(0),
        ':' if chars.get(1..3) == Some(&[':', ':']) => Some(0),
        '$' if chars.get(1) == Some(&'$') => Some(0),
        '0'..='9' => {
            let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
            matches!(chars.get(digits), Some('.' | ')')).then_some(digits)
        }
        _ => None,
    }
}

/// A link whose text is its own URL is written as `<url>` when that reads
/// back the same way.
fn autolink<'a>(url: &'a str, title: &str, content: &[Inline]) -> Option<&'a str> {
    let [Inline::Text { text }] = content else {
        return None;
    };
    if !title.is_empty()
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '&'))
    {
        return None;
    }
    let scheme = url.split_once(':').map(|(s, _)| s)?;
    let scheme_ok = (2..=32).contains(&scheme.len())
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if text == url && scheme_ok && !url.contains('\\') {
        return Some(url);
    }
    let email = url.strip_prefix("mailto:")?;
    (text == email && is_commonmark_email(email)).then_some(email)
}

/// The e-mail address shape a `CommonMark` `<…>` autolink accepts.
fn is_commonmark_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.chars().all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c));
    let label_ok = |l: &str| {
        (1..=63).contains(&l.len())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    local_ok && domain.split('.').all(label_ok)
}

/// A link the autolink extension made from bare text (`https://…`,
/// `www.…`, an e-mail address) is written back bare when that is safe.
fn bare_autolink<'a>(url: &str, title: &str, content: &'a [Inline]) -> Option<&'a str> {
    let [Inline::Text { text }] = content else {
        return None;
    };
    let shape = text.starts_with("https://") && url == text
        || text.starts_with("http://") && url == text
        || text.starts_with("www.") && url.strip_prefix("http://") == Some(text)
        || !text.contains(':') && url.strip_prefix("mailto:") == Some(text);
    let safe = text.chars().all(|c| c.is_ascii_alphanumeric() || "/:._~%+-?=&#@".contains(c))
        && text.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '/');
    (title.is_empty() && shape && safe).then_some(text.as_str())
}

fn longest_run(s: &str, c: char) -> usize {
    s.split(|x| x != c).map(str::len).max().unwrap_or(0)
}

/// Backslash-escapes `special` characters. `&` is written as `&#38;`
/// because destinations and titles decode entities after escapes.
fn push_escaped(out: &mut String, s: &str, special: &[char]) {
    for c in s.chars() {
        if c == '&' {
            out.push_str("&#38;");
            continue;
        }
        if special.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
}
