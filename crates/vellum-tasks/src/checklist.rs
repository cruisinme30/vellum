// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Checklist items in Vellum Markdown, read and edited in place.
//!
//! The Markdown block is the source of truth and the task row is a
//! projection (D19). This module finds every checklist item in a file and
//! makes minimal edits to it: ticking, rescheduling and rewriting fields
//! touch only the item's own line, so every other byte of the file, and
//! any concurrent edit merged around it, is left alone.
//!
//! An item is a list item whose text starts with `[ ]` or `[x]`:
//!
//! ```text
//! - [ ] Review the Northstar PRD @alice 2026-11-03 {p=1} ^blk-7f3a9k2m
//!   - [x] Read section 2 {done=2026-11-02T09:12:00Z} ^blk-0c4d1e2f
//! ```
//!
//! The title runs up to an optional attribute span ([`crate::attrs`]) and
//! an optional block ID. `@handle` mentions are the assignees. For items
//! written before the attribute span existed, the first ISO date in the
//! title is the deadline. Nested items are subtasks. Lines inside code,
//! data and math fences and the front matter are never items.

use std::ops::Range;

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use vellum_kernel::BlockId;

use crate::attrs::Attrs;
use crate::model::{Due, FieldError, Recurrence, TaskFields};
use crate::recur::next_occurrence;

/// One checklist item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Byte range of the item's first line, without the line break.
    pub line: Range<usize>,
    /// Byte range of the item with its continuation lines and subtasks,
    /// including the final line break.
    pub subtree: Range<usize>,
    /// Indentation in columns (a tab counts as four).
    pub indent: usize,
    /// Byte offset of the character between the brackets.
    pub check: usize,
    /// Whether the box is ticked.
    pub checked: bool,
    /// Byte range of everything after `] ` on the first line.
    pub tail: Range<usize>,
    /// The title, trimmed, without the attribute span or the block ID.
    pub title: String,
    /// The attribute span (empty when absent).
    pub attrs: Attrs,
    /// The task fields, with the inline-date fallback for `due` applied.
    pub fields: TaskFields,
    /// Why the attribute span's fields could not be read, if they could
    /// not. The item is still a task, with default fields.
    pub field_error: Option<FieldError>,
    /// The block ID, if the item has one.
    pub block: Option<BlockId>,
    /// `@handle` mentions in the title, without the `@`.
    pub mentions: Vec<String>,
    /// Index of the parent checklist item, for subtasks.
    pub parent: Option<usize>,
}

/// A file's checklist items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checklist {
    /// The items, in file order.
    pub items: Vec<Item>,
}

/// Why an edit could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// No checklist item has this block ID.
    #[error("no checklist item has block ID {0}")]
    NotFound(BlockId),
}

#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    end: usize,
    next: usize,
    indent: usize,
    blank: bool,
    /// Inside front matter or a fence (including the fence lines).
    literal: bool,
}

fn indent_of(text: &str) -> (usize, usize) {
    let mut columns = 0;
    let mut bytes = 0;
    for c in text.chars() {
        match c {
            ' ' => columns += 1,
            '\t' => columns += 4 - columns % 4,
            _ => break,
        }
        bytes += c.len_utf8();
    }
    (columns, bytes)
}

fn fence_marker(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == first).count();
    (run >= 3).then_some((first, run))
}

fn split_lines(source: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut math = false;
    let mut front_matter = source.starts_with("---\n") || source.starts_with("---\r\n");
    let mut first = true;
    while start < source.len() {
        let next = source
            .get(start..)
            .and_then(|rest| rest.find('\n'))
            .map_or(source.len(), |i| start + i + 1);
        let raw = source.get(start..next).unwrap_or_default();
        let content = raw.trim_end_matches(['\n', '\r']);
        let end = start + content.len();
        let (indent, indent_bytes) = indent_of(content);
        let trimmed = content.get(indent_bytes..).unwrap_or_default();
        let blank = trimmed.is_empty();
        let mut literal = front_matter || fence.is_some() || math;
        if front_matter && !first && (trimmed == "---" || trimmed == "...") {
            front_matter = false;
        } else if let Some((mark, run)) = fence {
            if fence_marker(trimmed).is_some_and(|(m, r)| m == mark && r >= run)
                && trimmed.trim_start_matches(mark).trim().is_empty()
            {
                fence = None;
            }
        } else if math {
            if trimmed.starts_with("$$") {
                math = false;
            }
        } else if !front_matter {
            if let Some(marker) = fence_marker(trimmed) {
                fence = Some(marker);
                literal = true;
            } else if trimmed == "$$" || (trimmed.starts_with("$$") && !trimmed[2..].contains("$$"))
            {
                math = true;
                literal = true;
            }
        }
        lines.push(Line {
            start,
            end,
            next,
            indent,
            blank,
            literal,
        });
        first = false;
        start = next;
    }
    lines
}

/// The parts of a list item line, if it is one.
struct ListLine {
    /// Byte offset just after the list marker and its spaces.
    content: usize,
    /// For checklist items: byte offset of the check character.
    check: Option<(usize, bool)>,
}

fn list_line(source: &str, line: &Line) -> Option<ListLine> {
    let content = source.get(line.start..line.end)?;
    let (_, indent_bytes) = indent_of(content);
    let rest = content.get(indent_bytes..)?;
    let marker_len = match rest.as_bytes().first()? {
        b'-' | b'*' | b'+' => 1,
        b'0'..=b'9' => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 9 {
                return None;
            }
            match rest.as_bytes().get(digits)? {
                b'.' | b')' => digits + 1,
                _ => return None,
            }
        }
        _ => return None,
    };
    let after_marker = rest.get(marker_len..)?;
    let spaces = after_marker.len() - after_marker.trim_start_matches([' ', '\t']).len();
    if spaces == 0 && !after_marker.is_empty() {
        return None;
    }
    let body_offset = line.start + indent_bytes + marker_len + spaces;
    let body = source.get(body_offset..line.end)?;
    let check = match body.as_bytes() {
        [b'[', c @ (b' ' | b'x' | b'X'), b']', rest @ ..]
            if rest.is_empty() || rest.first() == Some(&b' ') || rest.first() == Some(&b'\t') =>
        {
            Some((body_offset + 1, *c != b' '))
        }
        _ => None,
    };
    Some(ListLine {
        content: body_offset,
        check,
    })
}

const fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// `@handle` mentions, skipping e-mail addresses and the `@today` date.
#[must_use]
pub fn mentions(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    for (index, c) in text.char_indices() {
        if c == '@' && !previous.is_some_and(|p| p.is_alphanumeric() || p == '@') {
            let rest = text.get(index + 1..).unwrap_or_default();
            let handle: String = rest.chars().take_while(|c| is_handle_char(*c)).collect();
            let handle = handle.trim_end_matches(['.', '-']).to_ascii_lowercase();
            if !handle.is_empty() && handle != "today" && !found.contains(&handle) {
                found.push(handle);
            }
        }
        previous = Some(c);
    }
    found
}

/// The first `YYYY-MM-DD` date in the text, standing alone as a word.
#[must_use]
pub fn first_inline_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find_map(|i| {
        let candidate = text.get(i..i + 10)?;
        let before_ok = i == 0 || !bytes.get(i - 1)?.is_ascii_alphanumeric();
        let after_ok = bytes
            .get(i + 10)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'T');
        let shape = candidate.bytes().enumerate().all(|(j, b)| {
            if j == 4 || j == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
        (before_ok && after_ok && shape)
            .then(|| candidate.parse::<Date>().ok())
            .flatten()
    })
}

struct Tail {
    title: Range<usize>,
    attrs: Attrs,
    block: Option<BlockId>,
}

/// Splits an item's text into title, attribute span and block ID.
fn split_tail(text: &str) -> Tail {
    let mut end = text.trim_end().len();
    let mut block = None;
    let body = text.get(..end).unwrap_or_default();
    if let Some(pos) = body.rfind('^') {
        let candidate = body.get(pos + 1..).unwrap_or_default();
        let standalone = pos == 0 || body.get(..pos).is_some_and(|b| b.ends_with([' ', '\t']));
        if standalone && let Ok(id) = candidate.parse::<BlockId>() {
            block = Some(id);
            end = body.get(..pos).unwrap_or_default().trim_end().len();
        }
    }
    let mut attrs = Attrs::new();
    let body = text.get(..end).unwrap_or_default();
    if body.ends_with('}') {
        let mut search = body.len();
        while let Some(pos) = body.get(..search).and_then(|b| b.rfind('{')) {
            let span = body.get(pos..).unwrap_or_default();
            if let Ok(parsed) = Attrs::parse(span) {
                let before = body.get(..pos).unwrap_or_default();
                if before.is_empty() || before.ends_with([' ', '\t']) {
                    attrs = parsed;
                    end = before.trim_end().len();
                }
                break;
            }
            search = pos;
        }
    }
    let start = text.len() - text.trim_start().len();
    Tail {
        title: start.min(end)..end,
        attrs,
        block,
    }
}

impl Checklist {
    /// Finds every checklist item in a Vellum Markdown file.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        let lines = split_lines(source);
        let mut items: Vec<Item> = Vec::new();
        // Open list items: (indent, index into `items` when a checklist item).
        let mut stack: Vec<(usize, Option<usize>)> = Vec::new();
        let mut line_items: Vec<(usize, usize)> = Vec::new();
        for (line_index, line) in lines.iter().enumerate() {
            if line.blank {
                continue;
            }
            if line.literal {
                if line.indent == 0 {
                    stack.clear();
                }
                continue;
            }
            let Some(list) = list_line(source, line) else {
                if stack
                    .last()
                    .is_none_or(|(indent, _)| line.indent <= *indent)
                {
                    stack.clear();
                }
                continue;
            };
            while stack
                .last()
                .is_some_and(|(indent, _)| *indent >= line.indent)
            {
                let _ = stack.pop();
            }
            let parent = stack.last().and_then(|(_, item)| *item);
            let Some((check, checked)) = list.check else {
                stack.push((line.indent, None));
                continue;
            };
            let tail_start = (check + 2).min(line.end);
            let tail_text = source.get(tail_start..line.end).unwrap_or_default();
            let tail = split_tail(tail_text);
            let title = tail_text
                .get(tail.title.clone())
                .unwrap_or_default()
                .to_owned();
            let (mut fields, field_error) = match TaskFields::from_attrs(&tail.attrs) {
                Ok(fields) => (fields, None),
                Err(error) => (TaskFields::default(), Some(error)),
            };
            if fields.due.is_none() && tail.attrs.get("due").is_none() {
                fields.due = first_inline_date(&title).map(Due::Date);
            }
            let _ = list.content;
            stack.push((line.indent, Some(items.len())));
            line_items.push((line_index, items.len()));
            items.push(Item {
                line: line.start..line.end,
                subtree: line.start..line.next,
                indent: line.indent,
                check,
                checked,
                tail: tail_start..line.end,
                mentions: mentions(&title),
                title,
                attrs: tail.attrs,
                fields,
                field_error,
                block: tail.block,
                parent,
            });
        }
        for (line_index, item_index) in line_items {
            let Some(item) = items.get(item_index) else {
                continue;
            };
            let indent = item.indent;
            let mut end = lines.get(line_index).map_or(item.subtree.end, |l| l.next);
            for line in lines.iter().skip(line_index + 1) {
                if line.blank {
                    continue;
                }
                if line.indent <= indent {
                    break;
                }
                end = line.next;
            }
            if let Some(item) = items.get_mut(item_index) {
                item.subtree.end = end;
            }
        }
        Self { items }
    }

    /// The item with this block ID.
    #[must_use]
    pub fn find(&self, block: BlockId) -> Option<&Item> {
        self.items.iter().find(|i| i.block == Some(block))
    }

    fn index_of(&self, block: BlockId) -> Result<usize, EditError> {
        self.items
            .iter()
            .position(|i| i.block == Some(block))
            .ok_or(EditError::NotFound(block))
    }

    /// Indices of the item's descendants.
    fn descendants(&self, index: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut frontier = vec![index];
        while let Some(parent) = frontier.pop() {
            for (i, item) in self.items.iter().enumerate() {
                if item.parent == Some(parent) {
                    out.push(i);
                    frontier.push(i);
                }
            }
        }
        out.sort_unstable();
        out
    }
}

/// A set of byte-range replacements applied together.
#[derive(Debug, Default)]
struct Patch {
    edits: Vec<(Range<usize>, String)>,
}

impl Patch {
    fn replace(&mut self, range: Range<usize>, text: impl Into<String>) {
        self.edits.push((range, text.into()));
    }

    fn apply(mut self, source: &str) -> String {
        self.edits
            .sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut out = source.to_owned();
        for (range, text) in self.edits {
            if out.get(range.clone()).is_some() {
                out.replace_range(range, &text);
            }
        }
        out
    }
}

/// Writes an item's tail, everything after `]`: a space, then the title,
/// attribute span and block ID.
fn render_tail(title: &str, attrs: &Attrs, block: Option<BlockId>) -> String {
    let mut out = String::new();
    let title = title.trim();
    if !title.is_empty() {
        out.push(' ');
        out.push_str(title);
    }
    if !attrs.is_empty() {
        out.push(' ');
        out.push_str(&attrs.to_string());
    }
    if let Some(block) = block {
        out.push_str(" ^");
        out.push_str(&block.to_string());
    }
    out
}

fn format_done(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// What completing a task did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// The item was ticked.
    Done,
    /// A repeating item moved to its next occurrence and stays open.
    Rescheduled {
        /// The new anchor date.
        next: Date,
    },
}

fn rewrite(patch: &mut Patch, item: &Item, title: &str, fields: &TaskFields) {
    let mut attrs = item.attrs.clone();
    fields.write_attrs(&mut attrs);
    patch.replace(item.tail.clone(), render_tail(title, &attrs, item.block));
}

/// Moves a repeating item's fields to the next occurrence, or returns
/// `None` when the series is over.
fn advance(fields: &TaskFields, recurrence: &Recurrence, completed_on: Date) -> Option<TaskFields> {
    let anchor = fields.due.map(Due::date).or(fields.start)?;
    let next = next_occurrence(&recurrence.rule, recurrence.from, anchor, completed_on)?;
    let shift = anchor.until(next.date).ok()?;
    let mut fields = fields.clone();
    fields.due = fields.due.map(|due| due.on(next.date));
    fields.start = fields.start.and_then(|start| start.checked_add(shift).ok());
    fields.recurrence = Some(Recurrence {
        rule: next.rule,
        from: recurrence.from,
    });
    fields.done = None;
    Some(fields)
}

/// Ticks or unticks the item with this block ID; ticking records the
/// completion time in `done` and unticking removes it.
///
/// # Errors
///
/// Returns [`EditError::NotFound`] when no item has the block ID.
pub fn set_checked(
    source: &str,
    block: BlockId,
    checked: bool,
    at: Timestamp,
) -> Result<String, EditError> {
    let list = Checklist::parse(source);
    let item = list.find(block).ok_or(EditError::NotFound(block))?;
    let mut patch = Patch::default();
    patch.replace(item.check..item.check + 1, if checked { "x" } else { " " });
    let mut attrs = item.attrs.clone();
    attrs.set_opt("done", checked.then(|| format_done(at)));
    attrs.canonicalise();
    patch.replace(
        item.tail.clone(),
        render_tail(&item.title, &attrs, item.block),
    );
    Ok(patch.apply(source))
}

/// Completes the item: a repeating item moves to its next occurrence and
/// stays open; any other item is ticked. With `cascade`, open subtasks of
/// a ticked item are ticked too.
///
/// # Errors
///
/// Returns [`EditError::NotFound`] when no item has the block ID.
pub fn complete(
    source: &str,
    block: BlockId,
    now: &Zoned,
    cascade: bool,
) -> Result<(String, Completion), EditError> {
    let list = Checklist::parse(source);
    let index = list.index_of(block)?;
    let item = list.items.get(index).ok_or(EditError::NotFound(block))?;
    let mut patch = Patch::default();
    let at = now.timestamp();
    if let Some(recurrence) = &item.fields.recurrence
        && let Some(fields) = advance(&item.fields, recurrence, now.date())
    {
        let next = fields
            .due
            .map(Due::date)
            .or(fields.start)
            .unwrap_or_else(|| now.date());
        rewrite(&mut patch, item, &item.title, &fields);
        if item.checked {
            patch.replace(item.check..item.check + 1, " ");
        }
        // Subtasks of a repeating task reopen with it.
        for child in list.descendants(index) {
            if let Some(child) = list.items.get(child).filter(|c| c.checked) {
                patch.replace(child.check..child.check + 1, " ");
                let mut fields = child.fields.clone();
                fields.done = None;
                rewrite(&mut patch, child, &child.title, &fields);
            }
        }
        return Ok((patch.apply(source), Completion::Rescheduled { next }));
    }
    let mut targets = vec![index];
    if cascade {
        targets.extend(list.descendants(index));
    }
    for target in targets {
        if let Some(item) = list
            .items
            .get(target)
            .filter(|i| !i.checked || target == index)
        {
            patch.replace(item.check..item.check + 1, "x");
            let mut fields = item.fields.clone();
            if fields.done.is_none() {
                fields.done = Some(at);
            }
            rewrite(&mut patch, item, &item.title, &fields);
        }
    }
    Ok((patch.apply(source), Completion::Done))
}

/// Settles items ticked outside My Work: in the editor, in a Git commit or
/// in another app.
///
/// A ticked repeating item moves to its next occurrence and reopens, and a
/// ticked item without a `done` time gets one. Returns `None` when nothing
/// needs to change.
#[must_use]
pub fn settle(source: &str, now: &Zoned) -> Option<String> {
    let list = Checklist::parse(source);
    let mut patch = Patch::default();
    for item in list.items.iter().filter(|i| i.checked) {
        let recurring = item.fields.recurrence.as_ref().and_then(|recurrence| {
            let completed_on = item.fields.done.map_or_else(
                || now.date(),
                |t| t.to_zoned(now.time_zone().clone()).date(),
            );
            advance(&item.fields, recurrence, completed_on)
        });
        if let Some(fields) = recurring {
            patch.replace(item.check..item.check + 1, " ");
            rewrite(&mut patch, item, &item.title, &fields);
        } else if item.fields.done.is_none() && item.field_error.is_none() {
            let mut fields = item.fields.clone();
            fields.done = Some(now.timestamp());
            rewrite(&mut patch, item, &item.title, &fields);
        }
    }
    (!patch.edits.is_empty()).then(|| patch.apply(source))
}

/// Rewrites an item's title and fields, keeping its tick, unknown
/// attributes and block ID.
///
/// # Errors
///
/// Returns [`EditError::NotFound`] when no item has the block ID.
pub fn update(
    source: &str,
    block: BlockId,
    title: &str,
    fields: &TaskFields,
) -> Result<String, EditError> {
    let list = Checklist::parse(source);
    let item = list.find(block).ok_or(EditError::NotFound(block))?;
    let mut patch = Patch::default();
    let mut fields = fields.clone();
    // An inline date in the title already says the deadline; do not repeat
    // it in the span unless it differs.
    if fields.due.is_some()
        && item.attrs.get("due").is_none()
        && fields.due == first_inline_date(title).map(Due::Date)
    {
        fields.due = None;
    }
    rewrite(&mut patch, item, title, &fields);
    Ok(patch.apply(source))
}

/// Removes the item with its continuation lines and subtasks.
///
/// # Errors
///
/// Returns [`EditError::NotFound`] when no item has the block ID.
pub fn remove(source: &str, block: BlockId) -> Result<String, EditError> {
    let list = Checklist::parse(source);
    let item = list.find(block).ok_or(EditError::NotFound(block))?;
    let mut patch = Patch::default();
    patch.replace(item.subtree.clone(), "");
    Ok(patch.apply(source))
}

/// Writes one new checklist item line (without a line break).
#[must_use]
pub fn render_item(
    indent: usize,
    checked: bool,
    title: &str,
    fields: &TaskFields,
    block: BlockId,
) -> String {
    let mut attrs = Attrs::new();
    fields.write_attrs(&mut attrs);
    let check = if checked { 'x' } else { ' ' };
    format!(
        "{}- [{check}]{}",
        " ".repeat(indent),
        render_tail(title, &attrs, Some(block))
    )
}

/// Appends a new item: under `parent` as its last subtask, or at the end
/// of the file's last checklist (or of the file) when `parent` is `None`.
///
/// # Errors
///
/// Returns [`EditError::NotFound`] when the parent does not exist.
pub fn append(
    source: &str,
    parent: Option<BlockId>,
    title: &str,
    fields: &TaskFields,
    block: BlockId,
) -> Result<String, EditError> {
    let list = Checklist::parse(source);
    let (at, indent) = match parent {
        Some(parent) => {
            let item = list.find(parent).ok_or(EditError::NotFound(parent))?;
            (item.subtree.end, item.indent + 2)
        }
        None => list
            .items
            .iter()
            .rfind(|i| i.parent.is_none())
            .map_or((source.len(), 0), |i| (i.subtree.end, i.indent)),
    };
    let mut line = render_item(indent, fields.done.is_some(), title, fields, block);
    line.push('\n');
    let mut patch = Patch::default();
    let before = source.get(..at).unwrap_or_default();
    if !before.is_empty() && !before.ends_with('\n') {
        line.insert(0, '\n');
    }
    patch.replace(at..at, line);
    Ok(patch.apply(source))
}

/// Gives every checklist item without a block ID a fresh one, so each can
/// be linked to its task row. Returns `None` when all items have one.
#[must_use]
pub fn ensure_block_ids(source: &str) -> Option<String> {
    let list = Checklist::parse(source);
    let mut patch = Patch::default();
    let mut taken: Vec<BlockId> = list.items.iter().filter_map(|i| i.block).collect();
    for item in list.items.iter().filter(|i| i.block.is_none()) {
        let mut id = BlockId::generate();
        while taken.contains(&id) {
            id = BlockId::generate();
        }
        taken.push(id);
        let end = item.tail.end;
        let text = source.get(item.tail.clone()).unwrap_or_default();
        let trimmed = text.trim_end().len();
        patch.replace(item.tail.start + trimmed..end, format!(" ^{id}"));
    }
    (!patch.edits.is_empty()).then(|| patch.apply(source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const DOC: &str = r"---
id: 01M639NFH15EBMG6C7NX1ZK4KA
title: Launch
- [ ] not an item
---

# Launch plan

- [ ] Review the PRD @Alice 2026-11-03 ^blk-7f3a9k2m
  Notes stay with the item.
  - [x] Read section 2 {done=2026-11-02T09:12:00Z} ^blk-0c4d1e2f
  - [ ] Comment on risks {p=1} ^blk-1a2b3c4d
- [ ] Weekly sync {due=2026-11-02T10:00 rrule=FREQ=WEEKLY;BYDAY=MO} ^blk-5e6f7g8h
* plain bullet
  - [ ] Child of a plain bullet ^blk-9j0k1m2n

```markdown
- [ ] inside a fence ^blk-aaaaaaaa
```

1. [X] Numbered and done
";

    fn block(text: &str) -> BlockId {
        text.parse().unwrap()
    }

    #[test]
    fn finds_items_parents_and_fields() {
        let list = Checklist::parse(DOC);
        let titles: Vec<&str> = list.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Review the PRD @Alice 2026-11-03",
                "Read section 2",
                "Comment on risks",
                "Weekly sync",
                "Child of a plain bullet",
                "Numbered and done"
            ]
        );
        let review = &list.items[0];
        assert_eq!(review.mentions, ["alice"]);
        assert_eq!(review.fields.due, Some(Due::Date(date(2026, 11, 3))));
        assert_eq!(list.items[1].parent, Some(0));
        assert!(list.items[1].checked);
        assert_eq!(list.items[2].parent, Some(0));
        assert_eq!(list.items[3].parent, None);
        assert!(list.items[3].fields.recurrence.is_some());
        assert_eq!(list.items[4].parent, None);
        assert!(list.items[5].checked);
        assert_eq!(list.items[5].block, None);
        // The subtree of the first item covers its notes and subtasks.
        let subtree = &DOC[review.subtree.clone()];
        assert!(subtree.starts_with("- [ ] Review"));
        assert!(subtree.ends_with("^blk-1a2b3c4d\n"));
    }

    #[test]
    fn ticking_changes_only_the_item() {
        let at: Timestamp = "2026-11-03T08:00:00Z".parse().unwrap();
        let out = set_checked(DOC, block("blk-7f3a9k2m"), true, at).unwrap();
        assert!(out.contains(
            "- [x] Review the PRD @Alice 2026-11-03 {done=2026-11-03T08:00:00Z} ^blk-7f3a9k2m\n"
        ));
        assert_eq!(out.len(), DOC.len() + " {done=2026-11-03T08:00:00Z}".len());
        let back = set_checked(&out, block("blk-7f3a9k2m"), false, at).unwrap();
        assert_eq!(back, DOC);
        assert_eq!(
            set_checked(DOC, block("blk-zzzzzzzz"), true, at),
            Err(EditError::NotFound(block("blk-zzzzzzzz")))
        );
    }

    #[test]
    fn completing_a_repeating_item_moves_it() {
        let now: Zoned = "2026-11-02T11:00[Europe/London]".parse().unwrap();
        let (out, done) = complete(DOC, block("blk-5e6f7g8h"), &now, false).unwrap();
        assert_eq!(
            done,
            Completion::Rescheduled {
                next: date(2026, 11, 9)
            }
        );
        assert!(out.contains(
            "- [ ] Weekly sync {due=2026-11-09T10:00 rrule=FREQ=WEEKLY;BYDAY=MO} ^blk-5e6f7g8h\n"
        ));
    }

    #[test]
    fn completing_with_cascade_ticks_subtasks() {
        let now: Zoned = "2026-11-03T09:00[UTC]".parse().unwrap();
        let (out, done) = complete(DOC, block("blk-7f3a9k2m"), &now, true).unwrap();
        assert_eq!(done, Completion::Done);
        let list = Checklist::parse(&out);
        assert!(list.items[0].checked && list.items[1].checked && list.items[2].checked);
        // The earlier completion time of a done subtask is kept.
        assert!(out.contains("Read section 2 {done=2026-11-02T09:12:00Z}"));
        assert!(out.contains("Comment on risks {p=1 done=2026-11-03T09:00:00Z}"));
    }

    #[test]
    fn settle_handles_ticks_from_outside() {
        let ticked = DOC.replace("- [ ] Weekly sync", "- [x] Weekly sync");
        let now: Zoned = "2026-11-02T12:00[UTC]".parse().unwrap();
        let out = settle(&ticked, &now).unwrap();
        assert!(out.contains("- [ ] Weekly sync {due=2026-11-09T10:00"));
        assert!(out.contains("1. [X] Numbered and done {done=2026-11-02T12:00:00Z}"));
        assert_eq!(settle(&out, &now), None);
    }

    #[test]
    fn update_append_remove_and_ids() {
        let fields = TaskFields {
            due: Some(Due::Date(date(2026, 11, 3))),
            ..TaskFields::default()
        };
        let out = update(
            DOC,
            block("blk-7f3a9k2m"),
            "Review the PRD @alice 2026-11-03",
            &fields,
        )
        .unwrap();
        assert!(out.contains("- [ ] Review the PRD @alice 2026-11-03 ^blk-7f3a9k2m\n"));

        let out = append(
            DOC,
            Some(block("blk-7f3a9k2m")),
            "New subtask",
            &TaskFields::default(),
            block("blk-bbbbbbbb"),
        )
        .unwrap();
        assert!(out.contains("^blk-1a2b3c4d\n  - [ ] New subtask ^blk-bbbbbbbb\n- [ ] Weekly"));
        let out = append(
            "No list yet",
            None,
            "First",
            &TaskFields::default(),
            block("blk-cccccccc"),
        )
        .unwrap();
        assert_eq!(out, "No list yet\n- [ ] First ^blk-cccccccc\n");

        let out = remove(DOC, block("blk-7f3a9k2m")).unwrap();
        assert!(!out.contains("Read section 2"));
        assert!(out.contains("# Launch plan\n\n- [ ] Weekly sync"));

        let out = ensure_block_ids(DOC).unwrap();
        let list = Checklist::parse(&out);
        assert!(list.items.iter().all(|i| i.block.is_some()));
        assert_eq!(ensure_block_ids(&out), None);
    }

    #[test]
    fn helpers() {
        assert_eq!(
            mentions("ping @bob, @Ann.Lee and me@northstar.dev @today"),
            ["bob", "ann.lee"]
        );
        assert_eq!(first_inline_date("by 2026-11-03."), Some(date(2026, 11, 3)));
        assert_eq!(first_inline_date("v2026-11-03 or 2026-13-01"), None);
        let tail = split_tail("Title {bad span} {p=2} ^blk-7f3a9k2m");
        assert_eq!(tail.attrs.get("p"), Some("2"));
        assert_eq!(
            &"Title {bad span} {p=2} ^blk-7f3a9k2m"[tail.title],
            "Title {bad span}"
        );
    }
}
