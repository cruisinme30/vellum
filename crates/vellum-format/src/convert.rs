// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Parses a run of plain Markdown with comrak and converts comrak's arena
//! tree into the owned [`crate::ast`] model, in canonical form.

use comrak::nodes::{AlertType, AstNode, ListType, NodeValue, TableAlignment};
use comrak::{Arena, Options, parse_document};

use crate::ast::{
    Alignment, Attributes, Block, Container, Fence, Inline, List, ListItem,
    Table,
};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::extras;

/// The comrak extensions that make up the Markdown part of dialect v1.
fn options() -> Options<'static> {
    let mut options = Options::default();
    let ext = &mut options.extension;
    ext.strikethrough = true;
    ext.table = true;
    ext.autolink = true;
    ext.tasklist = true;
    ext.superscript = true;
    ext.subscript = true;
    ext.highlight = true;
    ext.wikilinks_title_after_pipe = true;
    ext.wikilinks_ignore_embeds = true;
    ext.alerts = true;
    ext.header_attributes = true;
    ext.fenced_code_attributes = true;
    // Escaped characters stay visible, so `\@team` is not a mention.
    options.parse.escaped_char_spans = true;
    options
}

/// Parses `src` as Markdown. `first_line` is the 1-based line of `src` in the
/// file, used for diagnostics; `depth` is the number of containers around it.
pub fn markdown(
    src: &str,
    first_line: usize,
    depth: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Block> {
    let arena = Arena::new();
    let root = parse_document(&arena, src, &options());
    let mut cx = Cx {
        line_offset: first_line.saturating_sub(1),
        depth,
        inline_depth: 0,
        too_deep: false,
        diagnostics,
    };
    cx.blocks(root)
}

/// Gives a table or block quote the ID written on its own line after it:
/// an ID-only paragraph that directly follows one without an ID.
fn attach_trailing_ids(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for block in blocks {
        if let Block::Paragraph {
            content,
            id: Some(id),
        } = &block
            && content.is_empty()
        {
            match out.last_mut() {
                Some(Block::Table(table)) if table.id.is_none() => {
                    table.id = Some(*id);
                    continue;
                }
                Some(Block::BlockQuote { id: quote_id, .. })
                    if quote_id.is_none() =>
                {
                    *quote_id = Some(*id);
                    continue;
                }
                _ => {}
            }
        }
        out.push(block);
    }
    out
}

/// Marks which lines of `src` belong to code blocks, as comrak reads the
/// whole text. Directive lines inside code are content, not directives.
pub fn code_lines(src: &str) -> Vec<bool> {
    let arena = Arena::new();
    // Alerts do not change what is code, and comrak's alert check is
    // quadratic in a run of `>` (see `prepass::space_quote_runs`).
    let mut options = options();
    options.extension.alerts = false;
    let root = parse_document(&arena, src, &options);
    let mut code = vec![false; src.split_inclusive('\n').count()];
    for node in root.descendants() {
        if let NodeValue::CodeBlock(_) = node.data().value {
            let pos = node.data().sourcepos;
            for line in pos.start.line..=pos.end.line {
                if let Some(flag) = code.get_mut(line.saturating_sub(1)) {
                    *flag = true;
                }
            }
        }
    }
    code
}

struct Cx<'d> {
    line_offset: usize,
    /// Block nesting around the node being converted.
    depth: usize,
    /// Inline nesting around the node being converted.
    inline_depth: usize,
    /// Whether the too-deep diagnostic has been reported.
    too_deep: bool,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl Cx<'_> {
    /// Converts the children of a block quote, callout or list item one
    /// level down, or flattens them to text past [`crate::MAX_NESTING`].
    fn nested_blocks<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Block> {
        if self.depth >= crate::MAX_NESTING {
            self.report_too_deep(parent);
            let text = descendant_text(parent);
            if text.is_empty() {
                return Vec::new();
            }
            let mut content = vec![Inline::Text { text }];
            let id = extras::take_block_id(&mut content);
            return vec![Block::Paragraph {
                content: extras::recognize(content),
                id,
            }];
        }
        self.depth += 1;
        let blocks = self.blocks(parent);
        self.depth -= 1;
        blocks
    }

    fn report_too_deep(&mut self, node: &AstNode<'_>) {
        if !self.too_deep {
            self.too_deep = true;
            let line = node.data().sourcepos.start.line + self.line_offset;
            self.diagnostics.push(crate::prepass::too_deep(line));
        }
    }

    fn blocks<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Block> {
        let blocks = parent
            .children()
            .flat_map(|node| self.block(node))
            .collect();
        attach_trailing_ids(blocks)
    }

    fn raw_html(&mut self, node: &AstNode<'_>) {
        let line = node.data().sourcepos.start.line + self.line_offset;
        self.diagnostics.push(Diagnostic::new(
            line,
            DiagnosticCode::RawHtml,
            "raw HTML is not part of Vellum Markdown; it is shown as text",
        ));
    }

    fn block<'a>(&mut self, node: &'a AstNode<'a>) -> Vec<Block> {
        let attrs = node
            .data()
            .attrs
            .as_deref()
            .map(convert_attrs)
            .unwrap_or_default();
        let value = node.data().value.clone();
        let block = match value {
            NodeValue::Paragraph => {
                let mut content = self.inlines(node);
                let id = extras::take_block_id(&mut content);
                Block::Paragraph {
                    content: extras::recognize(content),
                    id,
                }
            }
            NodeValue::Heading(heading) => {
                // A setext heading may span lines; an ATX heading cannot.
                let mut content =
                    trim_ends(extras::merge_text(unbreak(self.inlines(node))));
                let id = extras::take_block_id(&mut content);
                Block::Heading {
                    level: heading.level,
                    content: extras::recognize(content),
                    attrs,
                    id,
                }
            }
            NodeValue::ThematicBreak => Block::ThematicBreak,
            NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
                Block::BlockQuote {
                    blocks: self.nested_blocks(node),
                    id: None,
                }
            }
            NodeValue::Alert(alert) => {
                // Read the title as a container line would be read.
                let title = alert.title.unwrap_or_default();
                let (arg, attrs) = crate::attrs::split_trailing(title.trim());
                let (arg, id) = extras::split_block_id(arg);
                Block::Container(Container {
                    name: callout_name(alert.alert_type).to_owned(),
                    arg: arg.to_owned(),
                    attrs,
                    id,
                    sections: vec![self.nested_blocks(node)],
                })
            }
            NodeValue::List(list) => {
                let items: Vec<_> =
                    node.children().map(|item| self.item(item)).collect();
                // One item with at most one block has no blank line that
                // could show it is loose, so it is always tight.
                let single = matches!(
                    items.as_slice(),
                    [item] if item.blocks.len() <= 1
                );
                // A quote or table writes its ID after a blank line, which
                // makes the list loose, so such a list always is.
                let id_line =
                    items.iter().flat_map(|item| &item.blocks).any(|block| {
                        matches!(
                            block,
                            Block::BlockQuote { id: Some(_), .. }
                                | Block::Table(Table { id: Some(_), .. })
                        )
                    });
                Block::List(List {
                    start: (list.list_type == ListType::Ordered)
                        .then_some(list.start as u64),
                    tight: !id_line && (list.tight || single),
                    items,
                })
            }
            NodeValue::CodeBlock(code) => {
                let mut literal = code.literal;
                if !literal.is_empty() && !literal.ends_with('\n') {
                    literal.push('\n');
                }
                let (info, id) = extras::split_block_id(code.info.trim());
                let (lang, args) =
                    info.split_once(char::is_whitespace).unwrap_or((info, ""));
                if lang == "math" && args.trim().is_empty() && attrs.is_empty()
                {
                    Block::Math { literal, id }
                } else {
                    Block::Fence(Fence {
                        lang: lang.to_owned(),
                        args: args.trim().to_owned(),
                        attrs,
                        id,
                        literal,
                    })
                }
            }
            NodeValue::HtmlBlock(html) => {
                self.raw_html(node);
                return text_paragraphs(&html.literal);
            }
            NodeValue::Table(table) => self.table(node, &table.alignments),
            // Extensions that dialect v1 does not enable: keep their content.
            _ => return self.blocks(node),
        };
        vec![block]
    }

    fn item<'a>(&mut self, node: &'a AstNode<'a>) -> ListItem {
        let task = match &node.data().value {
            NodeValue::TaskItem(task) => Some(task.symbol.is_some()),
            _ => None,
        };
        if self.depth >= crate::MAX_NESTING {
            return ListItem {
                task,
                attrs: Attributes::default(),
                blocks: self.nested_blocks(node),
            };
        }
        self.depth += 1;
        let mut attrs = Attributes::default();
        let blocks = match (task, node.first_child()) {
            (Some(_), Some(first))
                if matches!(first.data().value, NodeValue::Paragraph) =>
            {
                // A task's `{…}` span (D82) ends its first paragraph, before
                // the block ID; it is read before dates and mentions are.
                let mut content = self.inlines(first);
                let mut id = extras::take_block_id(&mut content);
                if let Some(Inline::Text { text }) = content.last_mut() {
                    let (before, found) = crate::attrs::split_trailing(text);
                    if before.len() < text.trim_end().len() {
                        attrs = found;
                        *text = before.to_owned();
                        trim_end(&mut content);
                        // `text ^blk-… {}` writes the ID last, so read it
                        // again.
                        if id.is_none() {
                            id = extras::take_block_id(&mut content);
                        }
                    }
                }
                let mut blocks = Vec::new();
                if !content.is_empty() || id.is_some() || !attrs.is_empty() {
                    blocks.push(Block::Paragraph {
                        content: extras::recognize(extras::merge_text(content)),
                        id,
                    });
                }
                blocks.extend(
                    first
                        .following_siblings()
                        .skip(1)
                        .flat_map(|n| self.block(n)),
                );
                attach_trailing_ids(blocks)
            }
            _ => self.blocks(node),
        };
        self.depth -= 1;
        ListItem {
            task,
            attrs,
            blocks,
        }
    }

    fn table<'a>(
        &mut self,
        node: &'a AstNode<'a>,
        alignments: &[TableAlignment],
    ) -> Block {
        let mut rows = node.children().map(|row| {
            row.children()
                .map(|cell| extras::recognize(self.inlines(cell)))
                .collect::<Vec<_>>()
        });
        let header = rows.next().unwrap_or_default();
        Block::Table(Table {
            alignments: alignments
                .iter()
                .map(|a| convert_alignment(*a))
                .collect(),
            header,
            rows: rows.collect(),
            id: None,
        })
    }

    fn inlines<'a>(&mut self, parent: &'a AstNode<'a>) -> Vec<Inline> {
        if self.inline_depth >= crate::MAX_NESTING {
            self.report_too_deep(parent);
            return vec![Inline::Text {
                text: descendant_text(parent),
            }];
        }
        self.inline_depth += 1;
        let mut out = Vec::new();
        for node in parent.children() {
            self.inline(node, &mut out);
        }
        self.inline_depth -= 1;
        extras::merge_text(out)
    }

    fn inline<'a>(&mut self, node: &'a AstNode<'a>, out: &mut Vec<Inline>) {
        let value = node.data().value.clone();
        let inline = match value {
            NodeValue::Text(text) => Inline::Text {
                text: text.into_owned(),
            },
            NodeValue::Code(code) => Inline::Code { code: code.literal },
            NodeValue::SoftBreak => Inline::SoftBreak,
            NodeValue::LineBreak => Inline::HardBreak,
            NodeValue::HtmlInline(html) => {
                self.raw_html(node);
                out.extend(html_text(&html));
                return;
            }
            NodeValue::Emph => Inline::Emphasis {
                content: self.inlines(node),
            },
            NodeValue::Strong => Inline::Strong {
                content: self.inlines(node),
            },
            NodeValue::Strikethrough => Inline::Strikethrough {
                content: self.inlines(node),
            },
            NodeValue::Highlight => Inline::Highlight {
                content: self.inlines(node),
            },
            NodeValue::Subscript => Inline::Subscript {
                content: self.inlines(node),
            },
            NodeValue::Superscript => Inline::Superscript {
                content: self.inlines(node),
            },
            NodeValue::Link(link) => Inline::Link {
                url: link.url,
                title: link.title,
                content: self.inlines(node),
            },
            NodeValue::Image(link) => Inline::Image {
                url: link.url,
                title: link.title,
                alt: self.inlines(node),
            },
            NodeValue::Escaped => {
                let text: String = node
                    .children()
                    .filter_map(|child| match &child.data().value {
                        NodeValue::Text(text) => Some(text.to_string()),
                        _ => None,
                    })
                    .collect();
                Inline::Text {
                    text: extras::mark_escape(&text),
                }
            }
            NodeValue::WikiLink(link) => {
                let label = plain_text(&self.inlines(node))
                    .replace(extras::ESCAPED, "");
                Inline::WikiLink {
                    label: (label != link.url && !label.is_empty())
                        .then_some(label),
                    target: link.url,
                }
            }
            // Extensions that dialect v1 does not enable: keep their content.
            _ => {
                for child in node.children() {
                    self.inline(child, out);
                }
                return;
            }
        };
        out.push(inline);
    }
}

/// Raw inline HTML shown as text. A tag may span lines, but text cannot
/// hold a line ending: each one becomes the break it reads back as once
/// the text is escaped, with the whitespace around it dropped as at the
/// end and start of any paragraph line.
fn html_text(html: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut lines = html.split('\n').peekable();
    let mut first = true;
    while let Some(line) = lines.next() {
        let line = if first {
            line
        } else {
            line.trim_start_matches([' ', '\t'])
        };
        first = false;
        let last = lines.peek().is_none();
        let text = if last {
            line
        } else {
            line.trim_end_matches([' ', '\t'])
        };
        if !text.is_empty() {
            out.push(Inline::Text {
                text: text.to_owned(),
            });
        }
        if !last {
            let spaces = line.len() - line.trim_end_matches(' ').len();
            out.push(if spaces >= 2 {
                Inline::HardBreak
            } else {
                Inline::SoftBreak
            });
        }
    }
    out
}

/// Trims whitespace from the start of the first and end of the last text
/// run, as an ATX heading line is read.
fn trim_ends(mut content: Vec<Inline>) -> Vec<Inline> {
    if let Some(Inline::Text { text }) = content.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(Inline::Text { text }) = content.last_mut() {
        *text = text.trim_end().to_owned();
    }
    extras::merge_text(content)
}

/// Drops trailing whitespace and line breaks from inline content.
fn trim_end(content: &mut Vec<Inline>) {
    loop {
        match content.last_mut() {
            Some(Inline::SoftBreak | Inline::HardBreak) => {}
            Some(Inline::Text { text }) => {
                let len = text.trim_end().len();
                if len > 0 {
                    text.truncate(len);
                    return;
                }
            }
            _ => return,
        }
        content.truncate(content.len() - 1);
    }
}

/// Replaces line breaks with spaces, recursively.
fn unbreak(inlines: Vec<Inline>) -> Vec<Inline> {
    inlines
        .into_iter()
        .map(|inline| match inline {
            Inline::SoftBreak | Inline::HardBreak => Inline::Text {
                text: " ".to_owned(),
            },
            other => extras::map_children(other, unbreak),
        })
        .collect()
}

/// Maps `> [!type]` alerts onto the callout containers.
const fn callout_name(alert: AlertType) -> &'static str {
    match alert {
        AlertType::Note => "note",
        AlertType::Tip => "tip",
        AlertType::Important => "info",
        AlertType::Warning => "warn",
        AlertType::Caution => "crit",
    }
}

fn convert_attrs(attrs: &comrak::nodes::Attributes) -> Attributes {
    Attributes {
        id: attrs.id.clone(),
        classes: attrs.classes.clone(),
        pairs: attrs.pairs.clone(),
    }
}

const fn convert_alignment(alignment: TableAlignment) -> Alignment {
    match alignment {
        TableAlignment::None => Alignment::None,
        TableAlignment::Left => Alignment::Left,
        TableAlignment::Center => Alignment::Center,
        TableAlignment::Right => Alignment::Right,
    }
}

/// The text under a comrak node, gathered without recursion: text and
/// code as they are, line breaks and block boundaries as spaces.
fn descendant_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    for d in node.descendants().skip(1) {
        match &d.data().value {
            NodeValue::Text(text) => out.push_str(text),
            NodeValue::Code(code) => out.push_str(&code.literal),
            NodeValue::CodeBlock(code) => out.push_str(&code.literal),
            NodeValue::HtmlInline(html) => out.push_str(html),
            NodeValue::HtmlBlock(html) => out.push_str(&html.literal),
            NodeValue::SoftBreak
            | NodeValue::LineBreak
            | NodeValue::Paragraph
                if !out.is_empty() && !out.ends_with(' ') =>
            {
                out.push(' ');
            }
            _ => {}
        }
    }
    let collapsed: Vec<&str> = out.split_whitespace().collect();
    collapsed.join(" ")
}

/// The text of inline content, without markup.
#[must_use]
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text } => out.push_str(text),
            Inline::Code { code } => out.push_str(code),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            Inline::Emphasis { content }
            | Inline::Strong { content }
            | Inline::Strikethrough { content }
            | Inline::Highlight { content }
            | Inline::Subscript { content }
            | Inline::Superscript { content }
            | Inline::Link { content, .. }
            | Inline::Image { alt: content, .. } => {
                out.push_str(&plain_text(content));
            }
            Inline::WikiLink { target, label } => {
                out.push_str(label.as_deref().unwrap_or(target));
            }
            Inline::Mention { handle } => {
                out.push('@');
                out.push_str(handle);
            }
            Inline::IssueKey { key } => out.push_str(key),
            Inline::Date { date } if date == "today" => out.push_str("@today"),
            Inline::Date { date } => out.push_str(date),
        }
    }
    out
}

/// Raw HTML as literal text: one paragraph per run of non-blank lines.
fn text_paragraphs(literal: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut content: Vec<Inline> = Vec::new();
    for line in literal.lines().map(str::trim) {
        if line.is_empty() {
            if !content.is_empty() {
                blocks.push(Block::Paragraph {
                    content: std::mem::take(&mut content),
                    id: None,
                });
            }
            continue;
        }
        if !content.is_empty() {
            content.push(Inline::SoftBreak);
        }
        content.push(Inline::Text {
            text: line.to_owned(),
        });
    }
    if !content.is_empty() {
        blocks.push(Block::Paragraph { content, id: None });
    }
    // Read the text exactly as its escaped form will be read back.
    for block in &mut blocks {
        if let Block::Paragraph { content, id } = block {
            *id = extras::take_block_id(content);
            *content = extras::recognize(std::mem::take(content));
        }
    }
    blocks
}
