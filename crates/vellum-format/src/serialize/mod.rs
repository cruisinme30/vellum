// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The canonical serializer: writes a [`Document`] as Vellum Markdown.
//!
//! The output is a fixed point: `serialize(parse(serialize(d)))` equals
//! `serialize(d)`, and parsing it gives back the same tree. Canonical
//! choices: ATX headings, `-` bullets (`*` for a list that directly follows
//! another), `.` ordered markers (`)` likewise), backtick fences, `$$`
//! math, `:::` containers with one extra colon per level of nesting, and
//! one blank line between blocks.

mod inline;

use crate::ast::{Alignment, Block, Container, Document, Fence, List, Table};
use crate::{attrs, front_matter};
use inline::Place;

/// Writes `document` as canonical Vellum Markdown.
#[must_use]
#[tracing::instrument(level = "debug", skip_all, fields(blocks = document.blocks.len()))]
pub fn serialize(document: &Document) -> String {
    let mut out = String::new();
    if let Some(fm) = &document.front_matter {
        front_matter::write(fm, &mut out);
        if !document.blocks.is_empty() {
            out.push('\n');
        }
    }
    // Without front matter, a leading `---` would be read as front matter.
    let pos = Pos {
        nested: false,
        columns: false,
        file_start: document.front_matter.is_none(),
    };
    out.push_str(&blocks(&document.blocks, pos, "\n"));
    out
}

/// Where a run of blocks sits.
#[derive(Clone, Copy)]
struct Pos {
    /// Inside a list item or block quote: lines are indented or prefixed,
    /// so the line-start-only `:::` and `$$` syntax is not available.
    nested: bool,
    /// Directly inside a `columns` section, where `---` splits columns.
    columns: bool,
    /// At the very start of a file without front matter.
    file_start: bool,
}

/// Writes blocks, each ending in `\n`, joined by `separator`.
fn blocks(blocks: &[Block], pos: Pos, separator: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<&Block> = None;
    let mut previous_alternate = false;
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        let pos = Pos {
            file_start: pos.file_start && i == 0,
            ..pos
        };
        // A list right after another list needs a different marker, or the
        // two would be read as one.
        let alternate = matches!(
            (previous, block),
            (Some(Block::List(a)), Block::List(b)) if a.start.is_some() == b.start.is_some()
        ) && !previous_alternate;
        out.push_str(&self::block(block, pos, alternate));
        previous = Some(block);
        previous_alternate = alternate;
    }
    out
}

fn block(block: &Block, pos: Pos, alternate: bool) -> String {
    match block {
        Block::Paragraph { content, id } => {
            let mut text = inline::write(content, Place::Paragraph);
            push_block_id(&mut text, id.as_ref());
            text.push('\n');
            text
        }
        Block::Heading {
            level,
            content,
            attrs,
            id,
        } => {
            let mut text = "#".repeat(usize::from(*level));
            let body = inline::write(content, Place::Heading);
            if !body.is_empty() {
                text.push(' ');
                text.push_str(&body);
            }
            push_block_id(&mut text, id.as_ref());
            let attrs = attrs::to_string(attrs);
            if !attrs.is_empty() {
                text.push(' ');
                text.push_str(&attrs);
            }
            text.push('\n');
            text
        }
        Block::ThematicBreak if pos.columns || pos.file_start || pos.nested => "***\n".to_owned(),
        Block::ThematicBreak => "---\n".to_owned(),
        Block::BlockQuote { blocks: inner } => {
            let body = blocks(
                inner,
                Pos {
                    nested: true,
                    columns: false,
                    file_start: false,
                },
                "\n",
            );
            if body.is_empty() {
                ">\n".to_owned()
            } else {
                prefix_lines(&body, "> ", "> ")
            }
        }
        Block::List(list) => self::list(list, alternate),
        Block::Fence(fence) => self::fence(fence),
        Block::Math { literal } if !pos.nested && !literal.lines().any(|l| l == "$$") => {
            format!("$$\n{literal}$$\n")
        }
        Block::Math { literal } => self::fence(&Fence {
            lang: "math".to_owned(),
            args: String::new(),
            attrs: crate::ast::Attributes::default(),
            literal: literal.clone(),
        }),
        Block::Table(table) => self::table(table),
        Block::Container(container) if pos.nested => alert(container),
        Block::Container(container) => self::container(container),
    }
}

fn push_block_id(text: &mut String, id: Option<&vellum_kernel::BlockId>) {
    if let Some(id) = id {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push('^');
        text.push_str(&id.to_string());
    }
}

fn list(list: &List, alternate: bool) -> String {
    let separator = if list.tight { "" } else { "\n" };
    let mut out = String::new();
    for (i, item) in list.items.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        let mut marker = match list.start {
            None if alternate => "*".to_owned(),
            None => "-".to_owned(),
            Some(start) => {
                let n = start.saturating_add(u64::try_from(i).unwrap_or(u64::MAX));
                format!("{n}{}", if alternate { ')' } else { '.' })
            }
        };
        let indent = " ".repeat(marker.len() + 1);
        if let Some(done) = item.task {
            marker.push_str(if done { " [x]" } else { " [ ]" });
        }
        let pos = Pos {
            nested: true,
            columns: false,
            file_start: false,
        };
        let body = blocks(&item.blocks, pos, if list.tight { "" } else { "\n" });
        let paragraph_first = matches!(item.blocks.first(), Some(Block::Paragraph { .. }));
        if body.is_empty() {
            out.push_str(&marker);
            out.push('\n');
        } else if item.task.is_some() && !paragraph_first {
            // A task box must be followed by text on its line; other blocks
            // start on the next line.
            out.push_str(&marker);
            out.push_str(if list.tight { " \n" } else { " \n\n" });
            out.push_str(&prefix_lines(&body, &indent, &indent));
        } else {
            out.push_str(&prefix_lines(&body, &format!("{marker} "), &indent));
        }
    }
    out
}

fn fence(fence: &Fence) -> String {
    let c = if fence.lang.contains('`') || fence.args.contains('`') {
        '~'
    } else {
        '`'
    };
    let longest = fence
        .literal
        .lines()
        .map(|l| l.trim_start().chars().take_while(|x| *x == c).count())
        .max();
    let marks = c.to_string().repeat(longest.unwrap_or(0).max(2) + 1);
    let mut info = inline::escape_info(&fence.lang);
    for part in [
        inline::escape_info(&fence.args),
        attrs::to_string(&fence.attrs),
    ] {
        if !part.is_empty() {
            if !info.is_empty() {
                info.push(' ');
            }
            info.push_str(&part);
        }
    }
    // An info string starting with the fence character would lengthen it.
    let gap = if info.starts_with(c) { " " } else { "" };
    let mut out = format!("{marks}{gap}{info}\n{}", fence.literal);
    if !fence.literal.is_empty() && !fence.literal.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&marks);
    out.push('\n');
    out
}

fn table(table: &Table) -> String {
    let width = table.alignments.len().max(table.header.len()).max(1);
    let row = |cells: &[Vec<crate::ast::Inline>]| {
        let mut line = String::from("|");
        for i in 0..width {
            let cell = cells
                .get(i)
                .map(|c| inline::write(c, Place::Cell))
                .unwrap_or_default();
            line.push(' ');
            line.push_str(&cell);
            line.push_str(" |");
        }
        line.push('\n');
        line
    };
    let mut out = row(&table.header);
    out.push('|');
    for i in 0..width {
        out.push_str(match table.alignments.get(i) {
            Some(Alignment::Left) => " :-- |",
            Some(Alignment::Center) => " :-: |",
            Some(Alignment::Right) => " --: |",
            Some(Alignment::None) | None => " --- |",
        });
    }
    out.push('\n');
    for cells in &table.rows {
        out.push_str(&row(cells));
    }
    out
}

fn container(container: &Container) -> String {
    let colons = ":".repeat(3 + nesting_depth(container));
    let mut out = colons.clone();
    for part in [
        &container.name,
        &container.arg,
        &attrs::to_string(&container.attrs),
    ] {
        if !part.is_empty() {
            out.push(' ');
            out.push_str(part);
        }
    }
    out.push('\n');
    let columns = container.name == "columns";
    let pos = Pos {
        nested: false,
        columns,
        file_start: false,
    };
    let sections: Vec<String> = container
        .sections
        .iter()
        .map(|s| blocks(s, pos, "\n"))
        .collect();
    out.push_str(&sections.join("\n---\n\n"));
    out.push_str(&colons);
    out.push('\n');
    out
}

/// How many levels of `:::` containers sit inside this one.
fn nesting_depth(container: &Container) -> usize {
    container
        .sections
        .iter()
        .flatten()
        .filter_map(|b| match b {
            Block::Container(inner) => Some(1 + nesting_depth(inner)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// A callout inside a list item or quote, written as a `GitHub` alert.
fn alert(container: &Container) -> String {
    let kind = match container.name.as_str() {
        "tip" => "TIP",
        "info" => "IMPORTANT",
        "warn" => "WARNING",
        "crit" => "CAUTION",
        _ => "NOTE",
    };
    let mut head = format!("[!{kind}]");
    if !container.arg.is_empty() {
        head.push(' ');
        head.push_str(&container.arg);
    }
    head.push('\n');
    let pos = Pos {
        nested: true,
        columns: false,
        file_start: false,
    };
    let body: Vec<String> = container
        .sections
        .iter()
        .map(|s| blocks(s, pos, "\n"))
        .collect();
    let body = body.join("\n");
    let text = if body.is_empty() {
        head
    } else {
        format!("{head}{body}")
    };
    prefix_lines(&text, "> ", "> ")
}

/// Prefixes the first line with `first` and the others with `rest`;
/// blank lines get the prefix without trailing spaces.
fn prefix_lines(text: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        let prefix = if i == 0 { first } else { rest };
        if line.is_empty() {
            out.push_str(prefix.trim_end());
        } else {
            out.push_str(prefix);
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}
