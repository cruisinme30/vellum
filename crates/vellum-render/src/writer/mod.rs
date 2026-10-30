// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The AST walker that writes a page's HTML.
//!
//! All markup comes from literals in this module tree; user text goes
//! through [`crate::html`] escaping. SVG and MathML are collected as islands
//! and spliced in after the sanitising pass.

mod container;
mod fence;
mod inline;
mod issues;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use vellum_format::blocks::{Data, Read};
use vellum_format::{Alignment, Attributes, Block, Inline, List, Table};

use crate::figure::{Figure, FigureError, Figures};
use crate::html::{self, push_attr};
use crate::resolve::Resolver;
use crate::sanitize::xml::sanitize_island;

/// A heading in the page's outline.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TocEntry {
    /// 1 to 6.
    pub level: u8,
    /// The element id, for `#fragment` links.
    pub id: String,
    /// The heading text.
    pub text: String,
}

/// Something that rendered with a fallback; the page still shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Warning {
    /// Index of the top-level block it came from.
    pub block: usize,
    /// What happened, for the author.
    pub message: String,
}

/// Results of the async Kroki pre-pass, by content key.
pub type Resolved = HashMap<String, Result<Figure, FigureError>>;

pub struct Writer<'a> {
    pub out: String,
    pub islands: Vec<String>,
    pub toc: Vec<TocEntry>,
    pub warnings: Vec<Warning>,
    pub stale: usize,
    resolver: &'a dyn Resolver,
    figures: &'a Figures,
    kroki: &'a Resolved,
    ids: HashSet<String>,
    toc_marks: Vec<usize>,
    block: usize,
    depth: usize,
    issue_blocks: usize,
}

/// The prefix of every id taken from page content. A hyphen keeps the name
/// out of reach of `window.name`-style property access, and same-page links
/// (`#plan`) are rewritten to match.
pub const ID_PREFIX: &str = "u-";

/// Containers nested deeper than this render their body as plain text, so
/// a hostile document cannot exhaust the stack.
const MAX_DEPTH: usize = 32;

impl<'a> Writer<'a> {
    pub fn new(
        resolver: &'a dyn Resolver,
        figures: &'a Figures,
        kroki: &'a Resolved,
    ) -> Self {
        Self {
            out: String::with_capacity(16 * 1024),
            islands: Vec::new(),
            toc: Vec::new(),
            warnings: Vec::new(),
            stale: 0,
            resolver,
            figures,
            kroki,
            ids: HashSet::new(),
            toc_marks: Vec::new(),
            block: 0,
            depth: 0,
            issue_blocks: 0,
        }
    }

    pub fn document(&mut self, blocks: &[Block]) {
        self.out.push_str("<article class=\"vl-doc\">");
        for (i, block) in blocks.iter().enumerate() {
            self.block = i;
            self.block(block);
        }
        self.out.push_str("</article>");
        self.fill_tocs();
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(Warning {
            block: self.block,
            message: message.into(),
        });
    }

    /// Reserves a unique element id based on `wanted`.
    /// Keeps the typed data of a block, turning its diagnostics into
    /// warnings.
    pub fn data(&mut self, read: Option<Read<Data>>) -> Option<Data> {
        let read = read?;
        for d in read.diagnostics {
            self.warn(if d.line > 0 {
                format!("line {}: {}", d.line, d.message)
            } else {
                d.message
            });
        }
        Some(read.data)
    }

    /// A page-unique element id for `wanted`, always starting with
    /// [`ID_PREFIX`] so that no author-chosen id (`exports`, `module`,
    /// `location`, ...) can shadow a global through DOM clobbering.
    pub fn unique_id(&mut self, wanted: &str) -> String {
        let base = if wanted.is_empty() {
            format!("{ID_PREFIX}section")
        } else {
            format!("{ID_PREFIX}{wanted}")
        };
        let mut id = base.clone();
        let mut n = 2;
        while self.ids.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let _ = self.ids.insert(id.clone());
        id
    }

    /// Writes ` id="…" class="base user…"` from block attributes.
    pub fn attrs(&mut self, base: &str, attrs: &Attributes) {
        if let Some(id) = attrs.id.as_deref().filter(|id| html::is_user_id(id))
        {
            let id = self.unique_id(id);
            push_attr(&mut self.out, "id", &id);
        }
        let mut class = base.to_owned();
        for c in attrs.classes.iter().filter(|c| html::is_user_class(c)) {
            if !class.is_empty() {
                class.push(' ');
            }
            class.push_str(c);
        }
        if !class.is_empty() {
            push_attr(&mut self.out, "class", &class);
        }
    }

    /// Adds a sanitised SVG or MathML island and writes its placeholder.
    pub fn island(&mut self, markup: &str) -> Result<(), String> {
        let n = self.islands.len();
        let clean = sanitize_island(markup, &format!("f{n}"))
            .map_err(|e| e.to_string())?;
        self.islands.push(clean);
        self.out.push_str("<span data-vl-island=\"");
        self.out.push_str(&n.to_string());
        self.out.push_str("\"></span>");
        Ok(())
    }

    pub fn blocks(&mut self, blocks: &[Block]) {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.warn("blocks are nested too deeply; showing text");
            for b in blocks {
                self.out.push_str("<p>");
                html::text(&mut self.out, &plain_block_text(b));
                self.out.push_str("</p>");
            }
        } else {
            for b in blocks {
                self.block(b);
            }
        }
        self.depth -= 1;
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Paragraph { content, id } => {
                self.out.push_str("<p");
                if let Some(id) = id {
                    let id = self.unique_id(&id.to_string());
                    push_attr(&mut self.out, "id", &id);
                }
                self.out.push('>');
                self.inlines(content);
                self.out.push_str("</p>");
            }
            Block::Heading {
                level,
                content,
                attrs,
                id,
            } => {
                self.heading(*level, content, attrs, id.map(|b| b.to_string()))
            }
            Block::ThematicBreak => self.out.push_str("<hr>"),
            Block::BlockQuote { blocks, id } => {
                self.out.push_str("<blockquote");
                self.block_id(id.as_ref());
                self.out.push('>');
                self.blocks(blocks);
                self.out.push_str("</blockquote>");
            }
            Block::List(list) => self.list(list, ""),
            Block::Fence(f) => self.fence(f),
            Block::Math { literal, id } => {
                self.out.push_str("<div class=\"vl-math\"");
                self.block_id(id.as_ref());
                self.out.push('>');
                self.math(literal);
            }
            Block::Table(t) => self.table(t),
            Block::Container(c) => self.container(c),
        }
    }

    fn heading(
        &mut self,
        level: u8,
        content: &[Inline],
        attrs: &Attributes,
        block_id: Option<String>,
    ) {
        let level = level.clamp(1, 6);
        let text = vellum_format::plain_text(content);
        let wanted = attrs
            .id
            .clone()
            .filter(|id| html::is_user_id(id))
            .unwrap_or_else(|| html::slug(&text));
        let id = self.unique_id(&wanted);
        let tag = format!("h{level}");
        self.out.push('<');
        self.out.push_str(&tag);
        push_attr(&mut self.out, "id", &id);
        let user: Vec<&str> = attrs
            .classes
            .iter()
            .filter(|c| html::is_user_class(c))
            .map(String::as_str)
            .collect();
        if !user.is_empty() {
            push_attr(&mut self.out, "class", &user.join(" "));
        }
        if let Some(b) = block_id {
            push_attr(&mut self.out, "data-block", &b);
        }
        self.out.push('>');
        self.inlines(content);
        self.out.push_str("</");
        self.out.push_str(&tag);
        self.out.push('>');
        self.toc.push(TocEntry { level, id, text });
    }

    pub fn list(&mut self, list: &List, class: &str) {
        let tag = if list.start.is_some() { "ol" } else { "ul" };
        self.out.push('<');
        self.out.push_str(tag);
        if !class.is_empty() {
            push_attr(&mut self.out, "class", class);
        }
        if let Some(start) = list.start.filter(|s| *s != 1) {
            push_attr(&mut self.out, "start", &start.to_string());
        }
        self.out.push('>');
        for item in &list.items {
            match item.task {
                Some(done) => {
                    self.out.push_str(
                        "<li class=\"vl-task\"><input \
                            type=\"checkbox\" disabled",
                    );
                    if done {
                        self.out.push_str(" checked");
                    }
                    self.out.push('>');
                }
                None => self.out.push_str("<li>"),
            }
            self.item_blocks(&item.blocks, list.tight);
            self.out.push_str("</li>");
        }
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    /// Tight list items show their paragraphs without `<p>`.
    pub fn item_blocks(&mut self, blocks: &[Block], tight: bool) {
        if !tight {
            self.blocks(blocks);
            return;
        }
        for (i, b) in blocks.iter().enumerate() {
            match b {
                Block::Paragraph { content, .. } => {
                    if i > 0 {
                        self.out.push_str("<br>");
                    }
                    self.inlines(content);
                }
                other => self.blocks(std::slice::from_ref(other)),
            }
        }
    }

    /// Writes ` id="u-blk-…"` for a block id (the anchor for deep links and
    /// comments).
    fn block_id(&mut self, id: Option<&impl std::fmt::Display>) {
        if let Some(id) = id {
            let id = self.unique_id(&id.to_string());
            push_attr(&mut self.out, "id", &id);
        }
    }

    /// The body of a `vl-math` div, which the caller opens.
    fn math(&mut self, literal: &str) {
        let rendered =
            crate::math::to_mathml(literal).and_then(|m| self.island(&m));
        if let Err(e) = rendered {
            self.warn(format!("math: {e}"));
            self.out.push_str("<pre class=\"vl-math-error\"><code>");
            html::text(&mut self.out, literal.trim_end());
            self.out.push_str("</code></pre>");
        }
        self.out.push_str("</div>");
    }

    fn table(&mut self, table: &Table) {
        let compare = is_compare_matrix(table);
        self.out.push_str("<div class=\"vl-table-wrap\"><table");
        if compare {
            self.out.push_str(" class=\"vl-compare\"");
        }
        self.out.push_str("><thead><tr>");
        for (i, cell) in table.header.iter().enumerate() {
            self.out.push_str("<th scope=\"col\"");
            align(&mut self.out, table.alignments.get(i));
            self.out.push('>');
            self.inlines(cell);
            self.out.push_str("</th>");
        }
        self.out.push_str("</tr></thead><tbody>");
        for row in &table.rows {
            self.out.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                self.out.push_str("<td");
                align(&mut self.out, table.alignments.get(i));
                self.out.push('>');
                let text = vellum_format::plain_text(cell);
                match (compare && i > 0).then(|| compare_mark(&text)).flatten()
                {
                    Some((class, mark, label)) => {
                        self.out.push_str("<span class=\"");
                        self.out.push_str(class);
                        self.out.push_str("\" aria-label=\"");
                        self.out.push_str(label);
                        self.out.push_str("\">");
                        self.out.push_str(mark);
                        self.out.push_str("</span>");
                    }
                    None => match chip_class(&text) {
                        Some(class) => {
                            self.out.push_str("<span class=\"vl-chip ");
                            self.out.push_str(class);
                            self.out.push_str("\">");
                            self.inlines(cell);
                            self.out.push_str("</span>");
                        }
                        None => self.inlines(cell),
                    },
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>");
        }
        self.out.push_str("</tbody></table></div>");
    }

    /// Writes a TOC placeholder; [`Self::fill_tocs`] replaces it once every
    /// heading is known.
    pub fn toc_placeholder(&mut self, attrs: &Attributes) {
        let n = self.toc_marks.len();
        self.toc_marks.push(self.toc.len());
        self.out.push_str("<nav aria-label=\"Contents\"");
        self.attrs("vl-toc", attrs);
        self.out.push('>');
        let _ = write!(self.out, "\u{1}toc{n}\u{1}");
        self.out.push_str("</nav>");
    }

    fn fill_tocs(&mut self) {
        for (n, first) in self.toc_marks.clone().into_iter().enumerate() {
            let mut list = String::from("<ol>");
            let entries: Vec<&TocEntry> = self
                .toc
                .iter()
                .skip(first)
                .filter(|e| e.level >= 2)
                .collect();
            for e in &entries {
                list.push_str("<li class=\"vl-toc-l");
                list.push_str(&e.level.to_string());
                list.push_str("\"><a href=\"#");
                html::attr(&mut list, &e.id);
                list.push_str("\">");
                html::text(&mut list, &e.text);
                list.push_str("</a></li>");
            }
            list.push_str("</ol>");
            if entries.is_empty() {
                list = String::from("<p>No headings yet.</p>");
            }
            self.out =
                self.out.replacen(&format!("\u{1}toc{n}\u{1}"), &list, 1);
        }
    }

    /// The figure for a fence: drawn locally, or from the Kroki pre-pass.
    pub fn figure(
        &self,
        f: &vellum_format::Fence,
    ) -> Option<Result<Figure, FigureError>> {
        self.figures.draw_local(f).or_else(|| {
            let key = crate::figure::content_key(f);
            Some(match self.kroki.get(&key) {
                Some(r) => r.clone(),
                None => self.figures.kroki_cached(f, None),
            })
        })
    }

    pub const fn resolver(&self) -> &dyn Resolver {
        self.resolver
    }
}

fn align(out: &mut String, a: Option<&Alignment>) {
    match a {
        Some(Alignment::Center) => out.push_str(" class=\"vl-align-center\""),
        Some(Alignment::Right) => out.push_str(" class=\"vl-align-right\""),
        _ => {}
    }
}

/// `✓ ✗ ◐` or yes/no/part.
fn compare_mark(
    text: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match text.trim().to_lowercase().as_str() {
        "✓" | "✔" | "yes" => Some(("vl-yes", "✓", "Yes")),
        "✗" | "✘" | "no" => Some(("vl-no", "✗", "No")),
        "◐" | "part" | "partly" | "partial" => {
            Some(("vl-part", "◐", "Partly"))
        }
        _ => None,
    }
}

/// A GFM table whose body cells after the first column are all marks.
fn is_compare_matrix(table: &Table) -> bool {
    table.header.len() >= 2
        && !table.rows.is_empty()
        && table.rows.iter().all(|row| {
            row.iter().skip(1).all(|cell| {
                compare_mark(&vellum_format::plain_text(cell)).is_some()
            })
        })
}

/// Status words that become coloured chips in table cells.
fn chip_class(text: &str) -> Option<&'static str> {
    Some(match text.trim().to_lowercase().as_str() {
        "done" | "shipped" | "approved" | "stable" | "verified" | "yes"
        | "complete" | "completed" => "vl-chip-good",
        "in review" | "in progress" | "started" => "vl-chip-info",
        "partly" | "open" | "at risk" | "p1" | "p2" | "outdated" => {
            "vl-chip-warn"
        }
        "blocked" | "no" | "p0" | "off track" | "canceled" | "cancelled" => {
            "vl-chip-crit"
        }
        "decision" | "later" => "vl-chip-tip",
        "draft" | "backlog" => "vl-chip-note",
        _ => return None,
    })
}

/// A block's text, for the too-deep fallback.
fn plain_block_text(b: &Block) -> String {
    match b {
        Block::Paragraph { content, .. } | Block::Heading { content, .. } => {
            vellum_format::plain_text(content)
        }
        Block::Fence(f) => f.literal.clone(),
        Block::Math { literal, .. } => literal.clone(),
        Block::BlockQuote { blocks, .. } => blocks
            .iter()
            .map(plain_block_text)
            .collect::<Vec<_>>()
            .join(" "),
        Block::List(l) => l
            .items
            .iter()
            .flat_map(|i| i.blocks.iter().map(plain_block_text))
            .collect::<Vec<_>>()
            .join(" "),
        Block::Container(c) => c
            .sections
            .iter()
            .flatten()
            .map(plain_block_text)
            .collect::<Vec<_>>()
            .join(" "),
        Block::Table(_) | Block::ThematicBreak => String::new(),
    }
}
