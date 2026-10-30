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
        closing: None,
        follow: None,
        mark: None,
        edge: (false, false),
        next_visible: '\n',
        link_depth: 0,
    };
    // Adjacent marks of a kind merge, as they do when read, so their
    // delimiters never run together.
    w.inlines(&crate::extras::merge_text(inlines.to_vec()), None);
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
    /// The next sibling starts with `[`, so a trailing `!` would make it an
    /// image.
    bracket_next: bool,
    /// The first character of the enclosing mark's closing delimiter.
    closing: Option<char>,
    /// The first character written after the current inline, when it is a
    /// delimiter character that a run of the same character would join.
    follow: Option<char>,
    /// The delimiters around the content being written, when it is a mark's.
    mark: Option<MarkEdges>,
    /// Whether the current inline is the first and the last of its content.
    edge: (bool, bool),
    /// The character the parser sees after the current inline.
    next_visible: char,
    /// How many links and images enclose the current inline. The autolink
    /// extension does not run inside link text or image descriptions, so a
    /// link there is never bare.
    link_depth: usize,
}

/// What the parser sees around a mark's delimiters when it decides whether
/// they open and close: `~` and `=` are passed over.
#[derive(Clone, Copy)]
struct MarkEdges {
    /// The delimiter character.
    delimiter: char,
    /// The character before the opening delimiter.
    before: char,
    /// The character after the closing delimiter.
    after: char,
}

impl Writer {
    fn inlines(&mut self, inlines: &[Inline], parent_mark: Option<char>) {
        for (i, inline) in inlines.iter().enumerate() {
            // `***x***` reads as emphasis around strong, so emphasis that is
            // the only content of strong is written with `_`.
            let delim = if parent_mark == Some('S') && inlines.len() == 1 {
                '_'
            } else {
                '*'
            };
            let bare_ok = match inlines.get(i + 1) {
                None => self.bare_ok_at_end,
                Some(Inline::SoftBreak | Inline::HardBreak) => true,
                Some(Inline::Text { text }) => {
                    // The autolink extension leaves trailing punctuation out.
                    let rest =
                        text.trim_start_matches(['.', ',', ':', ';', '!', '?']);
                    rest.is_empty() || rest.starts_with(char::is_whitespace)
                }
                Some(_) => false,
            };
            self.bracket_next = matches!(
                inlines.get(i + 1),
                Some(Inline::Link { .. } | Inline::WikiLink { .. })
            );
            self.follow = inlines.get(i + 1).map_or(self.closing, opening_char);
            self.next_visible = inlines
                .get(i + 1)
                .map_or_else(|| self.after_content(), visible_first);
            self.edge = (i == 0, i + 1 == inlines.len());
            self.inline(inline, delim, bare_ok);
        }
    }

    fn inline(&mut self, inline: &Inline, delim: char, bare_ok: bool) {
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
                self.wrap(&delim.to_string(), content, Some('E'));
            }
            Inline::Strong { content } => {
                self.wrap("**", content, Some('S'));
            }
            Inline::Strikethrough { content } => self.wrap("~~", content, None),
            Inline::Highlight { content } => self.wrap("==", content, None),
            Inline::Subscript { content } => self.wrap("~", content, None),
            Inline::Superscript { content } => self.wrap("^", content, None),
            Inline::Link {
                url,
                title,
                content,
            } => {
                if let Some(bare) = bare_autolink(url, title, content).filter(|_| {
                    bare_ok
                        && self.link_depth == 0
                        && self.out.chars().next_back().is_none_or(char::is_whitespace)
                }) {
                    self.out.push_str(bare);
                } else if let Some(auto) = autolink(url, title, content) {
                    self.out.push('<');
                    self.out.push_str(auto);
                    self.out.push('>');
                } else {
                    self.out.push('[');
                    self.link_depth += 1;
                    self.enclosed(']', None, |w| w.inlines(content, None));
                    self.link_depth -= 1;
                    self.out.push(']');
                    self.destination(url, title);
                }
            }
            Inline::Image { url, title, alt } => {
                self.out.push_str("![");
                self.link_depth += 1;
                self.enclosed(']', None, |w| w.inlines(alt, None));
                self.link_depth -= 1;
                self.out.push(']');
                self.destination(url, title);
            }
            Inline::WikiLink { target, label } => {
                // Both parts are read with escapes and entities decoded,
                // and end at a bracket or a bar.
                let special = ['[', ']', '|', '\\'];
                self.out.push_str("[[");
                push_escaped(&mut self.out, target, &special);
                if let Some(label) = label {
                    self.out.push('|');
                    push_escaped(&mut self.out, label, &special);
                }
                self.out.push_str("]]");
            }
            Inline::Mention { handle } => {
                self.out.push('@');
                self.out.push_str(handle);
            }
            Inline::IssueKey { key } => self.out.push_str(key),
            Inline::Date { date } if date == "today" => {
                self.out.push_str("@today")
            }
            Inline::Date { date } => self.out.push_str(date),
        }
    }

    fn wrap(&mut self, mark: &str, content: &[Inline], emphasis: Option<char>) {
        let delimiter = mark.chars().next().unwrap_or(' ');
        let edges = MarkEdges {
            delimiter,
            before: self
                .out
                .trim_end_matches(['~', '='])
                .chars()
                .next_back()
                .unwrap_or('\n'),
            after: self.next_visible,
        };
        self.out.push_str(mark);
        let outer = self.bare_ok_at_end;
        self.bare_ok_at_end = true;
        self.enclosed(delimiter, Some(edges), |w| {
            w.inlines(content, emphasis);
        });
        self.bare_ok_at_end = outer;
        self.out.push_str(mark);
    }

    /// Writes content that a delimiter starting with `closing` follows.
    fn enclosed(
        &mut self,
        closing: char,
        mark: Option<MarkEdges>,
        write: impl FnOnce(&mut Self),
    ) {
        let outer = (self.closing.replace(closing), self.mark);
        self.mark = mark;
        write(self);
        (self.closing, self.mark) = outer;
    }

    /// The character the parser sees after the last inline of the content
    /// being written: the closing delimiter, or what follows it when that
    /// is passed over.
    const fn after_content(&self) -> char {
        match (self.mark, self.closing) {
            (Some(m), _) if matches!(m.delimiter, '~' | '=') => m.after,
            (Some(m), _) => m.delimiter,
            (None, Some(c)) => c,
            (None, None) => '\n',
        }
    }

    fn code(&mut self, code: &str) {
        let fence = "`".repeat(code_fence_len(code));
        let pad = code.starts_with('`')
            || code.ends_with('`')
            || (code.starts_with(' ')
                && code.ends_with(' ')
                && !code.trim().is_empty());
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
        let pointy = url.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '(' | ')' | '<' | '>')
        });
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
        let runs = self.escaped_runs(&chars, starts_line);
        let extras = crate::extras::extra_escapes(text);
        let marker_end = if starts_line && self.place != Place::Cell {
            line_start_escape(&chars)
        } else {
            None
        };
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
            let next = chars.get(i + 1).copied();
            let escape = match c {
                // A backslash is literal unless it comes before ASCII
                // punctuation or a line end; the next node is unknown here.
                '\\' => next.is_none_or(|n| n.is_ascii_punctuation()),
                '`' | '*' | '[' | ']' | '<' | '^' | '|' => true,
                '~' | '=' => runs.get(i).copied().unwrap_or(true),
                '_' => {
                    !(prev.is_some_and(char::is_alphanumeric)
                        && next.is_some_and(char::is_alphanumeric))
                }
                '&' => {
                    next.is_some_and(|n| n == '#' || n.is_ascii_alphabetic())
                }
                '!' => next.is_none() && self.bracket_next,
                '#' | '{' => self.place == Place::Heading,
                // Break up what the autolink extension would turn into links.
                ':' => chars.get(i + 1..i + 3) == Some(&['/', '/']),
                '.' => {
                    i >= 3
                        && chars.get(i - 3..i).is_some_and(|w| {
                            w.iter().all(|c| c.eq_ignore_ascii_case(&'w'))
                        })
                }
                '@' => prev.is_some_and(|p| {
                    p.is_alphanumeric() || matches!(p, '.' | '_' | '+' | '-')
                }),
                _ => false,
            } || (starts_line && Some(i) == marker_end)
                || extras.contains(&i);
            if escape {
                self.out.push('\\');
            }
            self.out.push(c);
        }
    }
}

impl Writer {
    /// Which `~` and `=` characters of `chars` to escape. A run of `~`, or
    /// of two or more `=`, could be read as a delimiter, so it is escaped
    /// whole. The parser also passes over these runs when it decides
    /// whether a delimiter next to them opens or closes (`p*~x*` is
    /// emphasis, `a ~~=~~ b` is not strikethrough), and a backslash stops
    /// that. So a run that starts or ends a mark's content is escaped,
    /// except where the backslash would keep the mark's delimiter from
    /// opening or closing between word characters. A run is always escaped
    /// where it would join a delimiter next to it or where it starts a line
    /// (a fence, a setext underline).
    fn escaped_runs(&self, chars: &[char], starts_line: bool) -> Vec<bool> {
        let mut escaped = Vec::with_capacity(chars.len());
        for run in chars.chunk_by(|a, b| a == b) {
            let (start, len) = (escaped.len(), run.len());
            let end = start + len;
            let c = run.first().copied().unwrap_or(' ');
            let escape = matches!(c, '~' | '=') && {
                let at_edge = self.mark.is_some()
                    && ((start == 0 && self.edge.0)
                        || (end == chars.len() && self.edge.1));
                let delimiter = c == '~' || len >= 2 || at_edge;
                let joins = (start == 0 && self.out.ends_with(c))
                    || (end == chars.len() && self.follow == Some(c));
                let line_start =
                    start == 0 && starts_line && self.place != Place::Cell;
                let flanks = self.mark.is_some_and(|m| {
                    (start == 0
                        && self.edge.0
                        && !matches!(m.delimiter, '^' | '~')
                        && is_word(m.before))
                        || (end == chars.len()
                            && self.edge.1
                            && is_word(m.after))
                });
                (delimiter && !flanks) || joins || line_start
            };
            escaped.resize(end, escape);
        }
        escaped
    }
}

/// The first character the parser sees for `inline` when it decides
/// whether a delimiter before it closes; `\\` stands for any punctuation.
fn visible_first(inline: &Inline) -> char {
    match inline {
        Inline::Text { text } => match text.chars().next() {
            Some('~' | '=') | None => '\\',
            Some(c) => c,
        },
        Inline::Strikethrough { content }
        | Inline::Subscript { content }
        | Inline::Highlight { content } => {
            content.first().map_or('\\', visible_first)
        }
        Inline::SoftBreak => '\n',
        Inline::Link {
            url,
            title,
            content,
        } => bare_autolink(url, title, content)
            .and_then(|t| t.chars().next())
            .unwrap_or('['),
        Inline::IssueKey { key: word } | Inline::Date { date: word }
            if word != "today" =>
        {
            word.chars().next().unwrap_or('\\')
        }
        _ => '\\',
    }
}

/// Neither white space nor punctuation, as the parser classes a character
/// next to a delimiter.
fn is_word(c: char) -> bool {
    !c.is_whitespace()
        && !c.is_ascii_punctuation()
        && (c.is_ascii() || c.is_alphanumeric() || c.is_control())
}

/// The first character written for `inline`, when a run of `~` or `=`
/// before it could join that character.
fn opening_char(inline: &Inline) -> Option<char> {
    match inline {
        Inline::Strikethrough { .. } | Inline::Subscript { .. } => Some('~'),
        Inline::Highlight { .. } => Some('='),
        Inline::Text { text } => text.chars().next(),
        _ => None,
    }
}

/// Which character of a line-leading text must be escaped so it is not
/// read as a block marker (heading, quote, list, setext line, table
/// delimiter row, container or math fence).
fn line_start_escape(chars: &[char]) -> Option<usize> {
    let first = *chars.first()?;
    match first {
        '#' | '>' | '-' | '+' | '=' => Some(0),
        ':' if chars.get(1..3) == Some(&[':', ':']) => Some(0),
        // `:-` would make the line a table delimiter row under the line
        // above; an escaped colon keeps it a paragraph line.
        ':' if chars.get(1) == Some(&'-') => Some(0),
        '$' if chars.get(1) == Some(&'$') => Some(0),
        '0'..='9' => {
            let digits =
                chars.iter().take_while(|c| c.is_ascii_digit()).count();
            matches!(chars.get(digits), Some('.' | ')')).then_some(digits)
        }
        _ => None,
    }
}

/// A link whose text is its own URL is written as `<url>` when that reads
/// back the same way.
fn autolink<'a>(
    url: &'a str,
    title: &str,
    content: &[Inline],
) -> Option<&'a str> {
    let [Inline::Text { text }] = content else {
        return None;
    };
    if !title.is_empty()
        || url.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '&')
        })
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
        && local.chars().all(|c| {
            c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c)
        });
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
fn bare_autolink<'a>(
    url: &str,
    title: &str,
    content: &'a [Inline],
) -> Option<&'a str> {
    let [Inline::Text { text }] = content else {
        return None;
    };
    let shape = ((text.starts_with("https://") || text.starts_with("http://"))
        && url == text)
        || (text.starts_with("www.")
            && url.strip_prefix("http://") == Some(text))
        || (!text.contains(':') && url.strip_prefix("mailto:") == Some(text));
    let safe = text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/:._~%+-?=&#@".contains(c))
        && text.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '/');
    (title.is_empty() && shape && safe).then_some(text.as_str())
}

/// The parser reads code spans with fences of at most this many backticks.
const MAX_CODE_FENCE: usize = 80;

/// A backtick fence for `code`: one longer than its longest backtick run,
/// or, when that is too long for the parser, the shortest length that is
/// not a run in the code (a run of the fence's length would close it).
fn code_fence_len(code: &str) -> usize {
    let runs: Vec<usize> = code
        .split(|c| c != '`')
        .map(str::len)
        .filter(|&n| n > 0)
        .collect();
    let longest = runs.iter().copied().max().unwrap_or(0);
    if longest < MAX_CODE_FENCE {
        return longest + 1;
    }
    (1..=MAX_CODE_FENCE)
        .find(|n| !runs.contains(n))
        .unwrap_or(MAX_CODE_FENCE)
}

/// Escapes a fence info string word, which is read with escapes and
/// entities decoded.
pub fn escape_info(s: &str) -> String {
    let mut out = String::new();
    push_escaped(&mut out, s, &['\\']);
    out
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
