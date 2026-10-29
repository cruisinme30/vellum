// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `SUMMARY.md`: order and nesting in mdBook or `GitBook` syntax (D14).
//!
//! The grammar is mdBook's: an optional `# Title`, then prefix chapters
//! (plain links), then numbered chapters (nested lists of links, split
//! into parts by `# Part` headings), then suffix chapters (plain links),
//! with `---` separators anywhere at the top level. `[Title]()` is a draft
//! with no file yet. Parsing uses pulldown-cmark, the parser mdBook itself
//! uses, so Vellum and mdBook always read a summary the same way.

use std::fmt::Write as _;
use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use crate::error::{BookError, Result};
use crate::path::ChapterPath;

/// The file name of the summary inside a book folder.
pub const FILE_NAME: &str = "SUMMARY.md";

/// The three top-level regions of a summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// Unnumbered chapters before the numbered ones (an introduction).
    Prefix,
    /// Numbered chapters, which may nest and be grouped into parts.
    Numbered,
    /// Unnumbered chapters after the numbered ones (appendices, glossary).
    Suffix,
}

/// One chapter link and the chapters nested under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// The title shown in navigation.
    pub name: String,
    /// The chapter file; `None` for a draft (`[Title]()`).
    pub location: Option<ChapterPath>,
    /// Nested chapters (numbered chapters only).
    pub children: Vec<Self>,
}

impl Chapter {
    /// A chapter with a file and no children.
    #[must_use]
    pub const fn new(name: String, location: ChapterPath) -> Self {
        Self {
            name,
            location: Some(location),
            children: Vec::new(),
        }
    }

    /// A draft chapter: a title with no file yet.
    #[must_use]
    pub const fn draft(name: String) -> Self {
        Self {
            name,
            location: None,
            children: Vec::new(),
        }
    }

    /// How many levels this chapter and its descendants span (1 for a leaf).
    #[must_use]
    pub fn height(&self) -> usize {
        1 + self.children.iter().map(Self::height).max().unwrap_or(0)
    }

    /// Visits this chapter and its descendants in reading order.
    pub fn walk<'a>(&'a self, visit: &mut impl FnMut(&'a Self)) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }
}

/// A top-level entry of a summary region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A chapter link (with nested chapters in the numbered region).
    Chapter(Chapter),
    /// A `---` separator.
    Separator,
    /// A `# Part` title (numbered region only).
    Part(String),
}

/// The parsed `SUMMARY.md`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// The `# Title` line, if any.
    pub title: Option<String>,
    /// Prefix chapters and separators.
    pub prefix: Vec<Item>,
    /// Numbered chapters, separators and part titles.
    pub numbered: Vec<Item>,
    /// Suffix chapters and separators.
    pub suffix: Vec<Item>,
}

/// The address of a chapter: its region, its index among the region's
/// items, then its index among each level of children.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Slot {
    /// The region.
    pub section: Section,
    /// The index at each level; never empty.
    pub path: Vec<usize>,
}

impl Slot {
    /// A top-level slot.
    #[must_use]
    pub fn top(section: Section, index: usize) -> Self {
        Self {
            section,
            path: vec![index],
        }
    }

    /// The nesting depth, 1 for a top-level chapter.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.path.len()
    }

    /// The slot of the parent chapter, if this one is nested.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (self.path.len() > 1).then(|| Self {
            section: self.section,
            path: self.path.get(..self.path.len() - 1).unwrap_or_default().to_vec(),
        })
    }

    /// The slot of this chapter's `index`-th child.
    #[must_use]
    pub fn child(&self, index: usize) -> Self {
        let mut path = self.path.clone();
        path.push(index);
        Self {
            section: self.section,
            path,
        }
    }

    /// Whether `other` is this slot or nested under it.
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        self.section == other.section && other.path.starts_with(&self.path)
    }

    /// The index within the parent.
    #[must_use]
    pub fn index(&self) -> usize {
        self.path.last().copied().unwrap_or_default()
    }
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.section)?;
        for index in &self.path {
            write!(f, "/{index}")?;
        }
        Ok(())
    }
}

impl Summary {
    /// The items of a region.
    #[must_use]
    pub const fn items(&self, section: Section) -> &Vec<Item> {
        match section {
            Section::Prefix => &self.prefix,
            Section::Numbered => &self.numbered,
            Section::Suffix => &self.suffix,
        }
    }

    /// The items of a region, mutably.
    pub const fn items_mut(&mut self, section: Section) -> &mut Vec<Item> {
        match section {
            Section::Prefix => &mut self.prefix,
            Section::Numbered => &mut self.numbered,
            Section::Suffix => &mut self.suffix,
        }
    }

    /// The top-level item at `slot` (any kind).
    #[must_use]
    pub fn item(&self, slot: &Slot) -> Option<&Item> {
        self.items(slot.section).get(*slot.path.first()?)
    }

    /// The chapter at `slot`.
    #[must_use]
    pub fn chapter(&self, slot: &Slot) -> Option<&Chapter> {
        let (first, rest) = slot.path.split_first()?;
        let Some(Item::Chapter(chapter)) = self.items(slot.section).get(*first) else {
            return None;
        };
        let mut chapter = chapter;
        for index in rest {
            chapter = chapter.children.get(*index)?;
        }
        Some(chapter)
    }

    /// The chapter at `slot`, mutably.
    pub fn chapter_mut(&mut self, slot: &Slot) -> Option<&mut Chapter> {
        let (first, rest) = slot.path.split_first()?;
        let Some(Item::Chapter(chapter)) = self.items_mut(slot.section).get_mut(*first) else {
            return None;
        };
        let mut chapter = chapter;
        for index in rest {
            chapter = chapter.children.get_mut(*index)?;
        }
        Some(chapter)
    }

    /// Removes and returns the item at `slot`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownPosition`] when nothing is there.
    pub fn remove(&mut self, slot: &Slot) -> Result<Item> {
        let missing = || BookError::UnknownPosition(slot.to_string());
        match slot.parent() {
            None => {
                let items = self.items_mut(slot.section);
                if slot.index() < items.len() {
                    Ok(items.remove(slot.index()))
                } else {
                    Err(missing())
                }
            }
            Some(parent) => {
                let parent = self.chapter_mut(&parent).ok_or_else(missing)?;
                if slot.index() < parent.children.len() {
                    Ok(Item::Chapter(parent.children.remove(slot.index())))
                } else {
                    Err(missing())
                }
            }
        }
    }

    /// Inserts `item` so that it ends up at `slot`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownPosition`] when the parent does not
    /// exist or the index is past the end, and [`BookError::NotAllowed`]
    /// when the item kind cannot live there (nesting outside the numbered
    /// region, separators or parts below the top level).
    pub fn insert(&mut self, slot: &Slot, item: Item) -> Result<()> {
        let missing = || BookError::UnknownPosition(slot.to_string());
        match (&item, slot.section) {
            (Item::Part(_), Section::Prefix | Section::Suffix) => {
                return Err(BookError::NotAllowed(
                    "part titles belong to the numbered chapters".to_owned(),
                ));
            }
            (Item::Chapter(chapter), Section::Prefix | Section::Suffix)
                if !chapter.children.is_empty() || slot.depth() > 1 =>
            {
                return Err(BookError::NotAllowed(
                    "prefix and suffix chapters cannot nest".to_owned(),
                ));
            }
            _ => {}
        }
        match slot.parent() {
            None => {
                let items = self.items_mut(slot.section);
                if slot.index() > items.len() {
                    return Err(missing());
                }
                items.insert(slot.index(), item);
            }
            Some(parent) => {
                let Item::Chapter(chapter) = item else {
                    return Err(BookError::NotAllowed(
                        "only chapters can be nested".to_owned(),
                    ));
                };
                let parent = self.chapter_mut(&parent).ok_or_else(missing)?;
                if slot.index() > parent.children.len() {
                    return Err(missing());
                }
                parent.children.insert(slot.index(), chapter);
            }
        }
        Ok(())
    }

    /// Every chapter with its slot, in reading order.
    #[must_use]
    pub fn chapters(&self) -> Vec<(Slot, &Chapter)> {
        fn visit<'a>(slot: &Slot, chapter: &'a Chapter, out: &mut Vec<(Slot, &'a Chapter)>) {
            for (index, child) in chapter.children.iter().enumerate() {
                let child_slot = slot.child(index);
                out.push((child_slot.clone(), child));
                visit(&child_slot, child, out);
            }
        }
        let mut out = Vec::new();
        for section in [Section::Prefix, Section::Numbered, Section::Suffix] {
            for (index, item) in self.items(section).iter().enumerate() {
                if let Item::Chapter(chapter) = item {
                    let slot = Slot::top(section, index);
                    out.push((slot.clone(), chapter));
                    visit(&slot, chapter, &mut out);
                }
            }
        }
        out
    }

    /// The slot of the chapter stored at `path`.
    #[must_use]
    pub fn find(&self, path: &ChapterPath) -> Option<Slot> {
        self.chapters()
            .into_iter()
            .find_map(|(slot, chapter)| (chapter.location.as_ref() == Some(path)).then_some(slot))
    }

    /// Parses `SUMMARY.md`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Summary`] with a line number when the text
    /// does not follow the mdBook grammar, and [`BookError::InvalidPath`]
    /// when a link points outside the book or not at a `.md` file.
    pub fn parse(text: &str) -> Result<Self> {
        SummaryParser::new(text).parse()
    }

    /// Writes the canonical `SUMMARY.md`, which mdBook parses back to the
    /// same structure.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let mut blocks: Vec<String> = Vec::new();
        if let Some(title) = &self.title {
            blocks.push(format!("# {}", escape(title)));
        }
        let push_region = |items: &[Item], numbered: bool, blocks: &mut Vec<String>| {
            let mut list = String::new();
            for item in items {
                match item {
                    Item::Chapter(chapter) if !numbered => {
                        blocks.push(link(chapter));
                    }
                    Item::Chapter(chapter) => write_list_item(&mut list, chapter, 0),
                    Item::Separator | Item::Part(_) => {
                        if !list.is_empty() {
                            blocks.push(std::mem::take(&mut list));
                        }
                        blocks.push(match item {
                            Item::Part(name) => format!("# {}", escape(name)),
                            _ => "---".to_owned(),
                        });
                    }
                }
            }
            if !list.is_empty() {
                blocks.push(list);
            }
        };
        push_region(&self.prefix, false, &mut blocks);
        push_region(&self.numbered, true, &mut blocks);
        push_region(&self.suffix, false, &mut blocks);
        for block in blocks {
            out.push_str(block.trim_end());
            out.push_str("\n\n");
        }
        let trimmed = out.trim_end().len();
        out.truncate(trimmed);
        out.push('\n');
        out
    }
}

fn write_list_item(out: &mut String, chapter: &Chapter, depth: usize) {
    let _ = writeln!(out, "{}- {}", "  ".repeat(depth), link(chapter));
    for child in &chapter.children {
        write_list_item(out, child, depth + 1);
    }
}

fn link(chapter: &Chapter) -> String {
    let dest = chapter.location.as_ref().map_or_else(String::new, |path| {
        let path = path.as_str();
        if path.contains(' ') || path.contains('(') || path.contains(')') {
            format!("<{path}>")
        } else {
            path.to_owned()
        }
    });
    format!("[{}]({dest})", escape(&chapter.name))
}

/// Escapes text so a `CommonMark` parser reads it back verbatim.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        if matches!(
            c,
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '&' | '!' | '~' | '|' | '#' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

struct SummaryParser<'a> {
    text: &'a str,
    events: Vec<(Event<'a>, Range<usize>)>,
    pos: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prefix,
    Numbered,
    Suffix,
}

impl<'a> SummaryParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            events: Parser::new(text).into_offset_iter().collect(),
            pos: 0,
        }
    }

    fn error(&self, message: impl Into<String>) -> BookError {
        let offset = self
            .events
            .get(self.pos)
            .or_else(|| self.events.last())
            .map_or(0, |(_, range)| range.start);
        let line = 1 + self
            .text
            .get(..offset)
            .unwrap_or_default()
            .matches('\n')
            .count();
        BookError::Summary {
            line,
            message: message.into(),
        }
    }

    fn next(&mut self) -> Option<Event<'a>> {
        let event = self.events.get(self.pos).map(|(event, _)| event.clone());
        self.pos += 1;
        event
    }

    fn peek(&self) -> Option<&Event<'a>> {
        self.events.get(self.pos).map(|(event, _)| event)
    }

    fn parse(mut self) -> Result<Summary> {
        let mut summary = Summary::default();
        let mut phase = Phase::Prefix;
        while let Some(event) = self.peek().cloned() {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    if level != HeadingLevel::H1 {
                        return Err(self.error("only `#` headings (title and parts) are allowed"));
                    }
                    self.pos += 1;
                    let name = self.text_until(TagEnd::Heading(level))?;
                    let empty = summary.prefix.is_empty() && summary.numbered.is_empty();
                    if summary.title.is_none() && empty && phase == Phase::Prefix {
                        summary.title = Some(name);
                    } else if phase == Phase::Suffix {
                        return Err(self.error("a part title cannot follow suffix chapters"));
                    } else {
                        phase = Phase::Numbered;
                        summary.numbered.push(Item::Part(name));
                    }
                }
                Event::Start(Tag::Paragraph) => {
                    self.pos += 1;
                    let links = self.paragraph_links()?;
                    if phase == Phase::Numbered {
                        phase = Phase::Suffix;
                    }
                    let target = if phase == Phase::Prefix {
                        &mut summary.prefix
                    } else {
                        &mut summary.suffix
                    };
                    target.extend(links.into_iter().map(Item::Chapter));
                }
                Event::Rule => {
                    self.pos += 1;
                    match phase {
                        Phase::Prefix => summary.prefix.push(Item::Separator),
                        Phase::Numbered => summary.numbered.push(Item::Separator),
                        Phase::Suffix => summary.suffix.push(Item::Separator),
                    }
                }
                Event::Start(Tag::List(_)) => {
                    if phase == Phase::Suffix {
                        return Err(self.error("numbered chapters cannot follow suffix chapters"));
                    }
                    phase = Phase::Numbered;
                    self.pos += 1;
                    let chapters = self.list()?;
                    summary
                        .numbered
                        .extend(chapters.into_iter().map(Item::Chapter));
                }
                _ => {
                    return Err(self.error(
                        "expected a link, a list of links, a `#` heading or a `---` separator",
                    ));
                }
            }
        }
        Ok(summary)
    }

    /// Collects plain text up to and including `end`.
    fn text_until(&mut self, end: TagEnd) -> Result<String> {
        let mut text = String::new();
        loop {
            match self.next() {
                Some(Event::End(tag)) if tag == end => break,
                Some(Event::Text(t) | Event::Code(t)) => text.push_str(&t),
                Some(Event::SoftBreak | Event::HardBreak) => text.push(' '),
                Some(Event::Start(Tag::Link { .. } | Tag::Image { .. })) => {
                    return Err(self.error("links are not allowed here"));
                }
                Some(_) => {}
                None => return Err(self.error("unexpected end of file")),
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err(self.error("empty title"));
        }
        Ok(text)
    }

    /// Reads the link at the cursor (after its `Start(Link)` event).
    fn link(&mut self, dest: &str) -> Result<Chapter> {
        let name = self.text_until(TagEnd::Link)?;
        if dest.is_empty() {
            Ok(Chapter::draft(name))
        } else {
            Ok(Chapter::new(name, ChapterPath::parse(dest)?))
        }
    }

    fn paragraph_links(&mut self) -> Result<Vec<Chapter>> {
        let mut links = Vec::new();
        loop {
            match self.next() {
                Some(Event::End(TagEnd::Paragraph)) => break,
                Some(Event::Start(Tag::Link { dest_url, .. })) => {
                    links.push(self.link(&dest_url)?);
                }
                Some(Event::SoftBreak | Event::HardBreak) => {}
                Some(Event::Text(t)) if t.trim().is_empty() => {}
                _ => {
                    self.pos = self.pos.saturating_sub(1);
                    return Err(self.error("prefix and suffix chapters must be plain links"));
                }
            }
        }
        Ok(links)
    }

    fn list(&mut self) -> Result<Vec<Chapter>> {
        let mut chapters = Vec::new();
        loop {
            match self.next() {
                Some(Event::End(TagEnd::List(_))) => break,
                Some(Event::Start(Tag::Item)) => chapters.push(self.item()?),
                _ => {
                    self.pos = self.pos.saturating_sub(1);
                    return Err(self.error("expected a list item"));
                }
            }
        }
        Ok(chapters)
    }

    fn item(&mut self) -> Result<Chapter> {
        let mut chapter: Option<Chapter> = None;
        loop {
            match self.next() {
                Some(Event::End(TagEnd::Item)) => break,
                Some(
                    Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) | Event::SoftBreak,
                ) => {}
                Some(Event::Text(t)) if t.trim().is_empty() => {}
                Some(Event::Start(Tag::Link { dest_url, .. })) if chapter.is_none() => {
                    chapter = Some(self.link(&dest_url)?);
                }
                Some(Event::Start(Tag::List(_))) => {
                    let Some(parent) = chapter.as_mut() else {
                        return Err(self.error("a nested list needs a parent link"));
                    };
                    let children = self.list()?;
                    parent.children.extend(children);
                }
                _ => {
                    self.pos = self.pos.saturating_sub(1);
                    return Err(self.error("each list item must be a single link"));
                }
            }
        }
        chapter.ok_or_else(|| self.error("each list item must be a single link"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRD: &str = "# Summary
[Introduction](README.md)
# Part I · Joining
- [Onboarding](onboarding/README.md)
  - [Your first week](onboarding/first-week.md)
  - [Security training]()
# Part II · How it works
- [Architecture](architecture/README.md)
  - [Services](architecture/services.md)
---
[Glossary](glossary.md)
";

    #[test]
    fn parses_the_prd_example() -> Result<()> {
        let summary = Summary::parse(PRD)?;
        assert_eq!(summary.title.as_deref(), Some("Summary"));
        assert_eq!(summary.prefix.len(), 1);
        assert_eq!(summary.suffix.len(), 1);
        assert!(matches!(summary.numbered.first(), Some(Item::Part(p)) if p == "Part I · Joining"));
        // The separator before the suffix belongs to the numbered region, as in mdBook.
        assert!(matches!(summary.numbered.last(), Some(Item::Separator)));
        let onboarding = summary
            .chapter(&Slot::top(Section::Numbered, 1))
            .ok_or_else(|| BookError::UnknownPosition("onboarding".into()))?;
        assert_eq!(onboarding.children.len(), 2);
        assert_eq!(onboarding.children.get(1).and_then(|c| c.location.clone()), None);
        Ok(())
    }

    #[test]
    fn canonical_form_round_trips() -> Result<()> {
        let summary = Summary::parse(PRD)?;
        let text = summary.to_markdown();
        assert_eq!(Summary::parse(&text)?, summary);
        assert_eq!(Summary::parse(&text)?.to_markdown(), text);
        Ok(())
    }

    #[test]
    fn reports_line_numbers() {
        let err = Summary::parse("# Summary\n\n- [A](a.md)\n\n[B](b.md)\n\n- [C](c.md)\n");
        assert!(
            matches!(err, Err(BookError::Summary { line: 7, .. })),
            "{err:?}"
        );
        assert!(Summary::parse("- [A](../a.md)\n").is_err());
        assert!(Summary::parse("- plain text\n").is_err());
        assert!(Summary::parse("## Sub\n").is_err());
    }

    #[test]
    fn escapes_special_titles() -> Result<()> {
        let mut summary = Summary::default();
        summary.numbered.push(Item::Chapter(Chapter::new(
            "Tags [beta] *and* `code` & <stuff> #1".to_owned(),
            ChapterPath::parse("my notes (draft).md")?,
        )));
        let text = summary.to_markdown();
        assert_eq!(Summary::parse(&text)?, summary);
        Ok(())
    }

    #[test]
    fn slots_insert_and_remove() -> Result<()> {
        let mut summary = Summary::parse(PRD)?;
        let slot = Slot {
            section: Section::Numbered,
            path: vec![1, 0],
        };
        let item = summary.remove(&slot)?;
        summary.insert(&Slot::top(Section::Numbered, 0), item)?;
        let first = summary
            .chapter(&Slot::top(Section::Numbered, 0))
            .map(|c| c.name.clone());
        assert_eq!(first.as_deref(), Some("Your first week"));
        let nested_prefix = Slot {
            section: Section::Prefix,
            path: vec![0, 0],
        };
        assert!(summary.insert(&nested_prefix, Item::Separator).is_err());
        Ok(())
    }
}
