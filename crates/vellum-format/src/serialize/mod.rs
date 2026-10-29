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
        after_dash: false,
    };
    out.push_str(&blocks(&document.blocks, pos, "\n"));
    out
}

/// Where a run of blocks sits.
#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one position, read in different places"
)]
struct Pos {
    /// Inside a list item or block quote: lines are indented or prefixed,
    /// so the line-start-only `:::` and `$$` syntax is not available.
    nested: bool,
    /// Directly inside a `columns` section, where `---` splits columns.
    columns: bool,
    /// At the very start of a file without front matter.
    file_start: bool,
    /// The first block shares its line with a `-` bullet, so a bullet list
    /// there must use `*`, or `- - -` would read as a thematic break.
    after_dash: bool,
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
            after_dash: pos.after_dash && i == 0,
            ..pos
        };
        // A list right after another list needs a different marker, or the
        // two would be read as one.
        let adjacent = matches!(
            (previous, block),
            (Some(Block::List(a)), Block::List(b)) if a.start.is_some() == b.start.is_some()
        ) && !previous_alternate;
        let alternate =
            adjacent || (pos.after_dash && matches!(block, Block::List(l) if l.start.is_none()));
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
        Block::BlockQuote { blocks: inner, id } => {
            let body = blocks(
                inner,
                Pos {
                    nested: true,
                    columns: false,
                    file_start: false,
                    after_dash: false,
                },
                "\n",
            );
            let mut out = if body.is_empty() {
                ">\n".to_owned()
            } else {
                prefix_lines(&body, "> ", "> ")
            };
            push_id_line(&mut out, id.as_ref());
            out
        }
        Block::List(list) => self::list(list, alternate),
        Block::Fence(fence) => self::fence(fence),
        Block::Math { literal, id } if !pos.nested && !literal.lines().any(|l| l == "$$") => {
            let mut open = "$$".to_owned();
            push_block_id(&mut open, id.as_ref());
            format!("{open}\n{literal}$$\n")
        }
        Block::Math { literal, id } => self::fence(&Fence {
            lang: "math".to_owned(),
            args: String::new(),
            attrs: crate::ast::Attributes::default(),
            id: *id,
            literal: literal.clone(),
        }),
        Block::Table(table) => {
            let mut out = self::table(table);
            push_id_line(&mut out, table.id.as_ref());
            out
        }
        Block::Container(container) if pos.nested => alert(container),
        Block::Container(container) => self::container(container),
    }
}

/// Writes a table's or quote's block ID on its own line after a blank one.
fn push_id_line(out: &mut String, id: Option<&vellum_kernel::BlockId>) {
    if let Some(id) = id {
        out.push_str("\n^");
        out.push_str(&id.to_string());
        out.push('\n');
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
            after_dash: marker == "-",
        };
        let separator = if list.tight { "" } else { "\n" };
        let body = match (item.task, item.blocks.split_first()) {
            (Some(_), Some((Block::Paragraph { content, id }, rest))) => {
                let mut text = task_text(content, id.as_ref(), &item.attrs);
                if !rest.is_empty() {
                    text.push_str(separator);
                    let rest_pos = Pos {
                        after_dash: false,
                        ..pos
                    };
                    text.push_str(&blocks(rest, rest_pos, separator));
                }
                text
            }
            _ => blocks(&item.blocks, pos, separator),
        };
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

/// A task item's first paragraph: its text, the `{…}` span (D82), then
/// the block ID.
fn task_text(
    content: &[crate::ast::Inline],
    id: Option<&vellum_kernel::BlockId>,
    attrs: &crate::ast::Attributes,
) -> String {
    let mut text = inline::write(content, Place::Paragraph);
    let attrs = attrs_after(&text, attrs);
    if !attrs.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&attrs);
    }
    push_block_id(&mut text, id);
    text.push('\n');
    text
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
    let args = inline::escape_info(&fence.args);
    if !args.is_empty() {
        if !info.is_empty() {
            info.push(' ');
        }
        info.push_str(&args);
    }
    push_block_id(&mut info, fence.id.as_ref());
    let attrs = attrs_after(&info, &fence.attrs);
    if !attrs.is_empty() {
        if !info.is_empty() {
            info.push(' ');
        }
        info.push_str(&attrs);
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
    let mut arg = container.arg.clone();
    push_block_id(&mut arg, container.id.as_ref());
    for part in [&container.name, &arg, &attrs_after(&arg, &container.attrs)] {
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
        after_dash: false,
    };
    let sections: Vec<String> = container
        .sections
        .iter()
        .map(|s| blocks(s, pos, "\n"))
        .collect();
    let mut body = sections.join("\n---\n\n");
    // The parser drops one empty column before the first split, so an
    // empty first column needs a split of its own.
    if sections.len() > 1 && sections.first().is_some_and(String::is_empty) {
        body.insert_str(0, "---\n\n");
    }
    // Blank lines around the body keep lists and fences apart from the
    // `:::` lines, as markdownlint (MD031, MD032) expects.
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
        out.push('\n');
    }
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

/// The attributes written after `text`. When there are none but `text`
/// itself ends in something that reads as attributes, an empty `{}` keeps
/// that text from being taken as them.
fn attrs_after(text: &str, attrs: &crate::ast::Attributes) -> String {
    let written = attrs::to_string(attrs);
    if written.is_empty() && attrs::split_trailing(text).0 != text.trim_end() {
        "{}".to_owned()
    } else {
        written
    }
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
    let mut arg = container.arg.clone();
    push_block_id(&mut arg, container.id.as_ref());
    for part in [&arg, &attrs_after(&arg, &container.attrs)] {
        if !part.is_empty() {
            head.push(' ');
            head.push_str(part);
        }
    }
    head.push('\n');
    let pos = Pos {
        nested: true,
        columns: false,
        file_start: false,
        after_dash: false,
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
