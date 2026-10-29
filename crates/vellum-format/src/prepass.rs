// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The directive pre-pass: finds `:::` containers and `$$` math blocks at
//! the start of lines, outside code blocks, and hands the Markdown between
//! them to comrak.
//!
//! A container opens with three or more colons, a name, an optional
//! argument and optional `{…}` attributes. A line of three or more colons
//! alone closes the innermost open container; outer containers are written
//! with more colons so the nesting is easy to read. Inside `columns`, a
//! `---` line starts the next column.

use crate::ast::{Block, Container, ContainerKind};
use crate::attrs;
use crate::convert;
use crate::diagnostic::{Diagnostic, DiagnosticCode};

/// One source line: its text (with the newline) and 1-based line number.
#[derive(Clone, Copy)]
struct Line<'a> {
    text: &'a str,
    number: usize,
    /// Inside a code block, where text is kept byte for byte.
    code: bool,
}

/// Parses `body`, whose first line is line `first_line` of the file.
pub fn blocks(body: &str, first_line: usize, diagnostics: &mut Vec<Diagnostic>) -> Vec<Block> {
    let code = convert::code_lines(body);
    let lines: Vec<Line<'_>> = body
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, text)| Line {
            text,
            number: first_line + i,
            code: code.get(i).copied().unwrap_or(false),
        })
        .collect();
    let mut scanner = Scanner {
        lines: &lines,
        code: &code,
        next: 0,
        depth: 0,
        too_deep: false,
        diagnostics,
    };
    let (mut sections, _) = scanner.scan(false, true);
    sections.pop().unwrap_or_default()
}

struct Scanner<'l, 'a, 'd> {
    lines: &'l [Line<'a>],
    /// Lines inside code blocks, where directives are not recognised.
    code: &'l [bool],
    next: usize,
    /// Containers open around the line being read.
    depth: usize,
    /// Whether the too-deep diagnostic has been reported.
    too_deep: bool,
    diagnostics: &'d mut Vec<Diagnostic>,
}

/// An opening `:::` line.
struct Opener {
    name: String,
    arg: String,
    attrs: crate::ast::Attributes,
    id: Option<vellum_kernel::BlockId>,
}

impl Scanner<'_, '_, '_> {
    /// Scans until a closing colon line (when `nested`) or the end. Returns
    /// the sections and whether a closing line was found.
    fn scan(&mut self, split_columns: bool, top: bool) -> (Vec<Vec<Block>>, bool) {
        let mut sections = vec![Vec::new()];
        let mut run: Vec<Line<'_>> = Vec::new();
        while let Some(&line) = self.lines.get(self.next) {
            let in_code = self.code.get(self.next).copied().unwrap_or(false);
            self.next += 1;
            let bare = line.text.trim_end_matches(['\n', '\r']);
            if in_code {
                run.push(line);
                continue;
            }
            let colons = bare.len() - bare.trim_start_matches(':').len();
            if colons >= 3 && bare.trim_start_matches(':').trim().is_empty() {
                if !top {
                    self.flush(&mut run, &mut sections);
                    return (sections, true);
                }
            } else if colons >= 3
                && let Some(opener) = parse_opener(bare.get(colons..).unwrap_or_default())
            {
                if self.depth >= crate::MAX_NESTING {
                    // Too deep: the opening line is read as text.
                    self.report_too_deep(line.number);
                    run.push(line);
                    continue;
                }
                self.flush(&mut run, &mut sections);
                let block = self.container(opener, line.number);
                push(&mut sections, block);
                continue;
            } else if let Some(id) = math_opener(bare)
                && let Some(math) = self.math(id)
            {
                self.flush(&mut run, &mut sections);
                push(&mut sections, math);
                continue;
            } else if split_columns && bare.trim_end() == "---" {
                self.flush(&mut run, &mut sections);
                sections.push(Vec::new());
                continue;
            }
            run.push(line);
        }
        self.flush(&mut run, &mut sections);
        (sections, false)
    }

    fn container(&mut self, opener: Opener, line: usize) -> Block {
        let kind = ContainerKind::from_name(&opener.name);
        if kind == ContainerKind::Unknown {
            self.diagnostics.push(Diagnostic::new(
                line,
                DiagnosticCode::UnknownContainer,
                format!(
                    "`::: {}` is not a Vellum container; its content is shown as is",
                    opener.name
                ),
            ));
        }
        self.depth += 1;
        let (mut sections, closed) = self.scan(kind == ContainerKind::Columns, false);
        self.depth -= 1;
        if !closed {
            self.diagnostics.push(Diagnostic::new(
                line,
                DiagnosticCode::UnclosedContainer,
                "container has no closing `:::` line; it runs to the end of its parent",
            ));
        }
        // A column split before any content does not make an empty column.
        if sections.len() > 1 && sections.first().is_some_and(Vec::is_empty) {
            let _ = sections.remove(0);
        }
        Block::Container(Container {
            name: opener.name,
            arg: opener.arg,
            attrs: opener.attrs,
            id: opener.id,
            sections,
        })
    }

    /// Reads a `$$` block after its opening line. Without a closing `$$`
    /// line it is not math, and the opening line is read as Markdown.
    fn math(&mut self, id: Option<vellum_kernel::BlockId>) -> Option<Block> {
        let rest = self.lines.get(self.next..)?;
        let end = rest
            .iter()
            .position(|l| l.text.trim_end_matches(['\n', '\r']) == "$$")?;
        let literal: String = rest.get(..end)?.iter().map(|l| l.text).collect();
        self.next += end + 1;
        let mut literal = literal;
        if !literal.is_empty() && !literal.ends_with('\n') {
            literal.push('\n');
        }
        Some(Block::Math { literal, id })
    }

    fn report_too_deep(&mut self, line: usize) {
        if !self.too_deep {
            self.too_deep = true;
            self.diagnostics.push(too_deep(line));
        }
    }

    fn flush(&mut self, run: &mut Vec<Line<'_>>, sections: &mut [Vec<Block>]) {
        let Some(first) = run.first() else {
            return;
        };
        let text: String = run
            .iter()
            .map(|l| {
                if l.code {
                    std::borrow::Cow::Borrowed(l.text)
                } else {
                    space_quote_runs(l.text)
                }
            })
            .collect();
        let blocks = convert::markdown(&text, first.number, self.depth, self.diagnostics);
        if let Some(section) = sections.last_mut() {
            section.extend(blocks);
        }
        run.clear();
    }
}

/// Longest run of `>` handed to comrak unchanged.
const MAX_QUOTE_RUN: usize = 32;

/// Breaks runs of more than [`MAX_QUOTE_RUN`] `>` with a space.
///
/// comrak's alert check rescans the rest of a `>` run at every quote
/// level, so one line of 100 000 `>` takes seconds. A space between quote
/// markers changes nothing in Markdown, and content that deep is shown
/// as plain text anyway. Code lines never reach this function.
fn space_quote_runs(line: &str) -> std::borrow::Cow<'_, str> {
    let too_long = line
        .split(|c| c != '>')
        .any(|run| run.len() > MAX_QUOTE_RUN);
    if !too_long {
        return std::borrow::Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + line.len() / MAX_QUOTE_RUN);
    let mut run = 0;
    for c in line.chars() {
        if c == '>' {
            if run == MAX_QUOTE_RUN {
                out.push(' ');
                run = 0;
            }
            run += 1;
        } else {
            run = 0;
        }
        out.push(c);
    }
    std::borrow::Cow::Owned(out)
}

/// The diagnostic for content nested deeper than [`crate::MAX_NESTING`].
pub fn too_deep(line: usize) -> Diagnostic {
    Diagnostic::new(
        line,
        DiagnosticCode::NestingTooDeep,
        format!(
            "content nests more than {} levels deep; the deeper part is shown as plain text",
            crate::MAX_NESTING
        ),
    )
}

fn push(sections: &mut [Vec<Block>], block: Block) {
    if let Some(section) = sections.last_mut() {
        section.push(block);
    }
}

/// Reads a `$$` line that opens math, with an optional block ID:
/// `Some(None)` for `$$`, `Some(Some(id))` for `$$ ^blk-…`.
#[expect(
    clippy::option_option,
    reason = "the outer option says whether the line opens math, the inner one holds its ID"
)]
fn math_opener(line: &str) -> Option<Option<vellum_kernel::BlockId>> {
    let rest = line.strip_prefix("$$")?;
    if rest.is_empty() {
        return Some(None);
    }
    let id = rest.strip_prefix(' ')?.strip_prefix('^')?.parse().ok()?;
    Some(Some(id))
}

/// Parses the part of an opening line after the colons.
fn parse_opener(rest: &str) -> Option<Opener> {
    let rest = rest.trim();
    let (head, attrs) = attrs::split_trailing(rest);
    let (head, id) = crate::extras::split_block_id(head);
    let (name, arg) = head.split_once(char::is_whitespace).unwrap_or((head, ""));
    let valid_name = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !(valid_name || (name.is_empty() && (!attrs.is_empty() || id.is_some()))) {
        return None;
    }
    Some(Opener {
        name: name.to_owned(),
        arg: arg.trim().to_owned(),
        attrs,
        id,
    })
}
