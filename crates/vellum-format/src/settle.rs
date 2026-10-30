// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Settles emphasis-like markup into a form the canonical text can carry.
//!
//! The `CommonMark` delimiter rules pair `*`, `_`, `~`, `=` and `^` runs by
//! their lengths and neighbours, so long or mixed runs can build trees that
//! no escaped text reproduces: the delimiters the serializer writes run
//! together and pair differently (`**?***x*****r`). Parsing is the only
//! exact model of those rules, so inline content holding such markup is
//! written and read back, and what it reads as is the document. Content
//! that does not settle after a few rounds keeps its text without the
//! marks. Either way a diagnostic says the markup was read differently.

use crate::Block;
use crate::ast::Inline;
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::serialize::inline::{self, Place};

/// Rounds of writing and reading before the marks are dropped.
const ROUNDS: usize = 4;

/// Settles the inline content of `blocks`, recursively.
pub fn blocks(blocks: &mut [Block], diagnostics: &mut Vec<Diagnostic>) {
    for block in blocks {
        match block {
            Block::Paragraph { content, .. } => {
                settle(content, Place::Paragraph, diagnostics)
            }
            Block::Heading { content, .. } => {
                settle(content, Place::Heading, diagnostics)
            }
            Block::BlockQuote { blocks: inner, .. } => {
                self::blocks(inner, diagnostics)
            }
            Block::List(list) => {
                for item in &mut list.items {
                    self::blocks(&mut item.blocks, diagnostics);
                }
            }
            Block::Table(table) => {
                for cell in table
                    .header
                    .iter_mut()
                    .chain(table.rows.iter_mut().flatten())
                {
                    settle(cell, Place::Cell, diagnostics);
                }
            }
            Block::Container(container) => {
                for section in &mut container.sections {
                    self::blocks(section, diagnostics);
                }
            }
            Block::ThematicBreak | Block::Fence(_) | Block::Math { .. } => {}
        }
    }
}

fn settle(
    content: &mut Vec<Inline>,
    place: Place,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !content.iter().any(has_marks) {
        return;
    }
    let mut current = std::mem::take(content);
    let mut changed = false;
    for _ in 0..ROUNDS {
        match read_back(&current, place) {
            Some(read) if read == current => break,
            Some(read) => {
                current = read;
                changed = true;
            }
            None => {
                current = unmark(current);
                changed = true;
                break;
            }
        }
    }
    if changed && read_back(&current, place).as_ref() != Some(&current) {
        current = unmark(current);
    }
    if changed {
        diagnostics.push(Diagnostic::new(
            0,
            DiagnosticCode::AmbiguousMarkup,
            "emphasis delimiters here pair differently once written; \
             the markup is kept as its canonical text reads",
        ));
    }
    *content = current;
}

/// Writes `content` and reads it back in the same place, or `None` when
/// the text does not read back as content of that place.
fn read_back(content: &[Inline], place: Place) -> Option<Vec<Inline>> {
    let text = inline::write(content, place);
    let src = match place {
        Place::Paragraph => format!("{text}\n"),
        Place::Heading => format!("# {text}\n"),
        Place::Cell => format!("| {text} |\n| --- |\n"),
    };
    let mut blocks = crate::parse_unsettled(&src).document.blocks;
    match (place, blocks.pop(), blocks.is_empty()) {
        (
            Place::Paragraph,
            Some(Block::Paragraph { content, id: None }),
            true,
        )
        | (Place::Heading, Some(Block::Heading { content, .. }), true) => {
            Some(content)
        }
        (Place::Cell, Some(Block::Table(mut table)), true)
            if table.header.len() == 1 =>
        {
            table.header.pop()
        }
        _ => None,
    }
}

/// Whether `inline` is or holds emphasis-like markup.
fn has_marks(inline: &Inline) -> bool {
    match inline {
        Inline::Emphasis { .. }
        | Inline::Strong { .. }
        | Inline::Strikethrough { .. }
        | Inline::Highlight { .. }
        | Inline::Subscript { .. }
        | Inline::Superscript { .. } => true,
        Inline::Link { content, .. } | Inline::Image { alt: content, .. } => {
            content.iter().any(has_marks)
        }
        _ => false,
    }
}

/// Replaces emphasis-like markup with its content, recursively.
fn unmark(content: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(content.len());
    for inline in content {
        match inline {
            Inline::Emphasis { content }
            | Inline::Strong { content }
            | Inline::Strikethrough { content }
            | Inline::Highlight { content }
            | Inline::Subscript { content }
            | Inline::Superscript { content } => out.extend(unmark(content)),
            other => out.push(crate::extras::map_children(other, unmark)),
        }
    }
    crate::extras::merge_text(out)
}
