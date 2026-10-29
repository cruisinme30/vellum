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
}

/// Parses `body`, whose first line is line `first_line` of the file.
pub fn blocks(body: &str, first_line: usize, diagnostics: &mut Vec<Diagnostic>) -> Vec<Block> {
    let lines: Vec<Line<'_>> = body
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, text)| Line {
            text,
            number: first_line + i,
        })
        .collect();
    let code = convert::code_lines(body);
    let mut scanner = Scanner {
        lines: &lines,
        code: &code,
        next: 0,
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
        let (mut sections, closed) = self.scan(kind == ContainerKind::Columns, false);
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

    fn flush(&mut self, run: &mut Vec<Line<'_>>, sections: &mut [Vec<Block>]) {
        let Some(first) = run.first() else {
            return;
        };
        let text: String = run.iter().map(|l| l.text).collect();
        let blocks = convert::markdown(&text, first.number, self.diagnostics);
        if let Some(section) = sections.last_mut() {
            section.extend(blocks);
        }
        run.clear();
    }
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
