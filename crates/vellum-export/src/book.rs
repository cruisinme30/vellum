// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Book export: a whole book, one section, or a section and everything
//! under it, as one flow.
//!
//! The input mirrors the books engine's table of contents (`Toc`,
//! `TocItem` and `TocEntry`), with each section's parsed document. The
//! export:
//!
//! - gives each section a numbered heading whose level is its depth, so
//!   `2.3` is a level-2 heading, and shifts the section's own headings
//!   below it;
//! - rebases a section or subtree so it starts at level 1;
//! - regenerates the table of contents from those headings;
//! - turns cross-references (`vellum://doc/<id>` links, and block links
//!   `vellum://doc/<id>#<block>`) to sections in the export into internal
//!   links that print with the target's page number (`§2.3 Services,
//!   p. 14`). Links to sections left out of the export point at the
//!   workspace when a base URL is set, and are plain text otherwise.

use std::time::Instant;

use vellum_format::{Block, Document};

use crate::error::ExportWarning;
use crate::flow::{Layout, Node, Span};
use crate::lower::{LowerOptions, Lowered, Refs, Session, inline_text};
use crate::metrics;
use crate::ports::Ports;

/// One section of a book, with its children.
#[derive(Debug, Clone, PartialEq)]
pub struct BookSection {
    /// The section's document ID; `None` for a draft.
    pub id: Option<String>,
    /// The navigation title.
    pub title: String,
    /// The number in the book's style (`2.3`, `II.C`); `None` for prefix
    /// and suffix chapters.
    pub number_label: Option<String>,
    /// The cross-reference label, such as `§2.3 Services`.
    pub label: String,
    /// Nesting depth, 1 at the top.
    pub depth: usize,
    /// A draft has no file yet and is left out.
    pub draft: bool,
    /// The section's content; `None` for a draft.
    pub document: Option<Document>,
    /// Nested sections.
    pub children: Vec<Self>,
}

impl BookSection {
    fn find(&self, id: &str) -> Option<&Self> {
        if self.id.as_deref() == Some(id) {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }

    fn flatten<'a>(&'a self, out: &mut Vec<&'a Self>) {
        out.push(self);
        for child in &self.children {
            child.flatten(out);
        }
    }
}

/// A top-level line of a book's contents.
#[derive(Debug, Clone, PartialEq)]
pub enum BookItem {
    /// A section and its children.
    Section(Box<BookSection>),
    /// A part title, which starts a new page.
    Part(String),
    /// A separator in the navigation; nothing in an export.
    Separator,
}

/// What to export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookScope {
    /// Every section.
    Whole,
    /// One section without its children.
    Section(String),
    /// One section and everything under it.
    Subtree(String),
}

/// A book export request.
#[derive(Debug, Clone, PartialEq)]
pub struct BookExport {
    /// The book title.
    pub title: String,
    /// The contents, in reading order.
    pub items: Vec<BookItem>,
    /// What to export.
    pub scope: BookScope,
    /// Levels shown in the regenerated table of contents.
    pub toc_depth: u8,
}

/// One step of the export, in reading order.
enum Step<'a> {
    /// A part title on a new page.
    Part(String),
    /// A section's heading and content.
    Section {
        section: &'a BookSection,
        level: u8,
        prefix: String,
        new_page: bool,
    },
}

/// Lowers a book, or part of one, into one flow with [`Layout::Book`].
///
/// `opts` gives the target, the date live blocks are frozen at and the
/// base URL; the heading offset, anchor prefix and references are set
/// per section.
///
/// A scope naming a section that is not in the book exports only the
/// title page and adds a warning.
#[tracing::instrument(
    name = "export.book",
    skip_all,
    fields(scope = ?book.scope)
)]
pub async fn lower_book(
    book: &BookExport,
    ports: &Ports,
    opts: &LowerOptions,
) -> Lowered {
    let started = Instant::now();
    let mut session = Session::new(ports);
    let (title, steps) = plan(book, &mut session);

    // Every section gets its anchor before any is lowered, so a reference
    // to a later section resolves.
    let mut refs = Refs::default();
    for step in &steps {
        if let Step::Section {
            section, prefix, ..
        } = step
        {
            if let Some(id) = &section.id {
                let _ = refs.docs.insert(
                    id.clone(),
                    (format!("{prefix}top"), section.label.clone()),
                );
            }
            if let Some(doc) = &section.document {
                collect_block_refs(
                    &doc.blocks,
                    prefix,
                    &section.label,
                    &mut refs,
                );
            }
        }
    }

    let mut blocks = vec![Node::Toc {
        depth: book.toc_depth.clamp(1, 6),
    }];
    for step in &steps {
        match step {
            Step::Part(part) => {
                blocks.push(Node::PageBreak);
                blocks.push(Node::Heading {
                    level: 1,
                    number: None,
                    spans: vec![Span::plain(part.clone())],
                    anchor: None,
                    outlined: true,
                });
            }
            Step::Section {
                section,
                level,
                prefix,
                new_page,
            } => {
                if *new_page {
                    blocks.push(Node::PageBreak);
                }
                let nodes = section_nodes(
                    &mut session,
                    section,
                    *level,
                    prefix,
                    opts,
                    &refs,
                )
                .await;
                blocks.extend(nodes);
            }
        }
    }
    let lowered = session.finish(title, Layout::Book, &opts.as_of, blocks);
    metrics::export_done("book", started, Ok(lowered.flow.blocks.len()));
    lowered
}

/// The export title and its steps. Depth becomes heading level, lifted so
/// the exported section starts at level 1; top-level sections start a new
/// page; drafts are left out.
fn plan<'a>(
    book: &'a BookExport,
    session: &mut Session<'_>,
) -> (String, Vec<Step<'a>>) {
    let mut steps = Vec::new();
    let mut count = 0usize;
    let mut push =
        |steps: &mut Vec<Step<'a>>, section: &'a BookSection, shift: usize| {
            if section.draft || section.document.is_none() {
                return;
            }
            count += 1;
            let depth = section.depth.saturating_sub(shift).max(1);
            steps.push(Step::Section {
                section,
                level: u8::try_from(depth.min(6)).unwrap_or(6),
                prefix: format!("s{count}-"),
                new_page: depth == 1,
            });
        };
    match &book.scope {
        BookScope::Whole => {
            for item in &book.items {
                match item {
                    BookItem::Section(top) => {
                        let mut all = Vec::new();
                        top.flatten(&mut all);
                        for section in all {
                            push(&mut steps, section, 0);
                        }
                    }
                    BookItem::Part(title) => {
                        steps.push(Step::Part(title.clone()))
                    }
                    BookItem::Separator => {}
                }
            }
            (book.title.clone(), steps)
        }
        BookScope::Section(id) | BookScope::Subtree(id) => {
            let found = book.items.iter().find_map(|item| match item {
                BookItem::Section(section) => section.find(id),
                _ => None,
            });
            let Some(found) = found else {
                session.warn(ExportWarning::new(
                    "book",
                    format!("section {id} is not in this book"),
                ));
                return (book.title.clone(), steps);
            };
            let shift = found.depth.saturating_sub(1);
            if matches!(book.scope, BookScope::Section(_)) {
                push(&mut steps, found, shift);
            } else {
                let mut all = Vec::new();
                found.flatten(&mut all);
                for section in all {
                    push(&mut steps, section, shift);
                }
            }
            // The first page follows the title page directly.
            if let Some(Step::Section { new_page, .. }) = steps.first_mut() {
                *new_page = false;
            }
            (format!("{} — {}", book.title, found.title), steps)
        }
    }
}

/// The heading and content of one section.
async fn section_nodes(
    session: &mut Session<'_>,
    section: &BookSection,
    level: u8,
    prefix: &str,
    opts: &LowerOptions,
    refs: &Refs,
) -> Vec<Node> {
    let mut out = vec![Node::Heading {
        level,
        number: section.number_label.clone(),
        spans: vec![Span::plain(section.title.clone())],
        anchor: Some(format!("{prefix}top")),
        outlined: true,
    }];
    let Some(doc) = &section.document else {
        return out;
    };
    let mut body = doc.clone();
    if let Some(Block::Heading {
        level: 1, content, ..
    }) = body.blocks.first()
        && inline_text(content).trim() == section.title.trim()
    {
        let _ = body.blocks.remove(0);
    }
    let shallowest = body
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        })
        .min()
        .unwrap_or(1);
    let offset = level.saturating_add(1).saturating_sub(shallowest);
    let section_opts = LowerOptions {
        target: opts.target,
        as_of: opts.as_of.clone(),
        base_url: opts.base_url.clone(),
        heading_offset: offset,
        anchor_prefix: prefix.to_owned(),
        refs: refs.clone(),
    };
    out.extend(session.lower(&body, &section_opts).await);
    out
}

/// Registers block IDs of headings and paragraphs so block links resolve.
fn collect_block_refs(
    blocks: &[Block],
    prefix: &str,
    label: &str,
    refs: &mut Refs,
) {
    for block in blocks {
        match block {
            Block::Heading {
                content,
                id: Some(id),
                ..
            } => {
                let id = id.to_string();
                let _ = refs.blocks.insert(
                    id.clone(),
                    (format!("{prefix}{id}"), inline_text(content)),
                );
            }
            Block::Paragraph { id: Some(id), .. } => {
                let id = id.to_string();
                let _ = refs.blocks.insert(
                    id.clone(),
                    (format!("{prefix}{id}"), label.to_owned()),
                );
            }
            Block::BlockQuote { blocks, .. } => {
                collect_block_refs(blocks, prefix, label, refs)
            }
            Block::List(list) => {
                for item in &list.items {
                    collect_block_refs(&item.blocks, prefix, label, refs);
                }
            }
            Block::Container(container) => {
                for section in &container.sections {
                    collect_block_refs(section, prefix, label, refs);
                }
            }
            _ => {}
        }
    }
}
