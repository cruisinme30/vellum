// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! HTML to the Vellum document model, for Confluence and other HTML
//! exports.
//!
//! Headings, paragraphs, lists, tables, quotes, code, links, images and
//! inline styles map one to one. Confluence's information macros become
//! callouts, its expand macro a `details` container and its code macro a
//! fenced block with the language kept. Scripts, styles and forms are
//! dropped; anything else is read for its content.

use scraper::{ElementRef, Node};
use vellum_format::{
    Alignment, Attributes, Block, Container, Fence, Inline, List, ListItem,
    Table,
};

/// Deepest element nesting followed, to bound recursion on hostile input.
const MAX_DEPTH: usize = 64;

/// Converts the children of `root` to blocks.
#[must_use]
pub fn blocks(root: ElementRef<'_>) -> Vec<Block> {
    let mut out = Out::default();
    children(root, &mut out, 0);
    out.flush();
    out.blocks
}

#[derive(Default)]
struct Out {
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
}

impl Out {
    fn flush(&mut self) {
        let content = tidy(std::mem::take(&mut self.inlines));
        if !content.is_empty() {
            self.blocks.push(Block::Paragraph { content, id: None });
        }
    }

    fn block(&mut self, block: Block) {
        self.flush();
        self.blocks.push(block);
    }
}

fn children(element: ElementRef<'_>, out: &mut Out, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for child in element.children() {
        match child.value() {
            Node::Text(text) => push_text(&mut out.inlines, text),
            Node::Element(_) => {
                if let Some(child) = ElementRef::wrap(child) {
                    node(child, out, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn has_class(element: ElementRef<'_>, class: &str) -> bool {
    element.value().classes().any(|c| c == class)
}

fn class_starting(element: ElementRef<'_>, prefix: &str) -> Option<String> {
    element
        .value()
        .classes()
        .find_map(|c| c.strip_prefix(prefix).map(str::to_owned))
}

#[allow(clippy::too_many_lines)] // one arm per element
fn node(element: ElementRef<'_>, out: &mut Out, depth: usize) {
    let name = element.value().name();
    match name {
        "script" | "style" | "noscript" | "template" | "head" | "form"
        | "button" | "input" | "select" | "textarea" | "iframe" | "object"
        | "svg" => {}
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name.get(1..).and_then(|n| n.parse().ok()).unwrap_or(2);
            let content = tidy(inline_children(element, depth));
            if !content.is_empty() {
                out.block(Block::Heading {
                    level,
                    content,
                    attrs: Attributes::default(),
                    id: None,
                });
            }
        }
        "p" => {
            out.flush();
            let mut inner = Out::default();
            children(element, &mut inner, depth);
            inner.flush();
            out.blocks.extend(inner.blocks);
        }
        "br" => out.inlines.push(Inline::HardBreak),
        "hr" => out.block(Block::ThematicBreak),
        "ul" | "ol" => {
            let start = (name == "ol").then(|| {
                element
                    .value()
                    .attr("start")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1)
            });
            let items: Vec<ListItem> = element
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|li| li.value().name() == "li")
                .map(|li| {
                    let task = li
                        .value()
                        .attr("data-inline-task-status")
                        .map(|s| s == "complete")
                        .or_else(|| {
                            has_class(li, "checked").then_some(true).or_else(
                                || has_class(li, "unchecked").then_some(false),
                            )
                        });
                    ListItem {
                        attrs: vellum_format::Attributes::default(),
                        task,
                        blocks: blocks_at(li, depth),
                    }
                })
                .collect();
            if !items.is_empty() {
                out.block(Block::List(List {
                    start,
                    tight: true,
                    items,
                }));
            }
        }
        "blockquote" => {
            let blocks = blocks_at(element, depth);
            out.block(Block::BlockQuote { id: None, blocks });
        }
        "pre" => {
            let lang = element
                .value()
                .attr("data-syntaxhighlighter-params")
                .and_then(|p| {
                    p.split(';').find_map(|kv| {
                        let (k, v) = kv.split_once(':')?;
                        (k.trim() == "brush").then(|| v.trim().to_owned())
                    })
                })
                .or_else(|| {
                    element
                        .select(&code_selector())
                        .next()
                        .and_then(|code| class_starting(code, "language-"))
                })
                .unwrap_or_default();
            let lang = match lang.as_str() {
                "none" | "text" | "plain" => String::new(),
                "js" | "jscript" => "javascript".into(),
                "py" => "python".into(),
                "sh" | "shell" => "bash".into(),
                _ => lang,
            };
            let mut literal: String = element.text().collect();
            if !literal.ends_with('\n') {
                literal.push('\n');
            }
            out.block(Block::Fence(Fence {
                id: None,
                lang,
                args: String::new(),
                attrs: Attributes::default(),
                literal,
            }));
        }
        "table" => {
            if let Some(table) = table(element, depth) {
                out.block(Block::Table(table));
            }
        }
        "img" => {
            if let Some(image) = image(element) {
                out.inlines.push(image);
            }
        }
        "div" | "section" | "aside" => {
            if let Some(kind) =
                class_starting(element, "confluence-information-macro-")
                    .filter(|k| !matches!(k.as_str(), "body" | "icon"))
            {
                let tone = match kind.as_str() {
                    "note" => "warn",
                    "warning" => "crit",
                    "tip" => "tip",
                    _ => "info",
                };
                let body = element.select(&body_selector()).next().map_or_else(
                    || blocks_at(element, depth),
                    |b| blocks_at(b, depth),
                );
                out.block(callout(tone, body));
            } else if has_class(element, "expand-container") {
                let title = element
                    .select(&expand_title_selector())
                    .next()
                    .map(|t| collapse(&t.text().collect::<String>()))
                    .unwrap_or_default();
                let body = element
                    .select(&expand_body_selector())
                    .next()
                    .map_or_else(Vec::new, |b| blocks_at(b, depth));
                out.block(Block::Container(Container {
                    id: None,
                    name: "details".into(),
                    arg: title,
                    attrs: Attributes::default(),
                    sections: vec![body],
                }));
            } else if has_class(element, "code") && has_class(element, "panel")
            {
                // A Confluence code macro: the `pre` inside carries the code.
                out.flush();
                children(element, out, depth);
                out.flush();
            } else {
                out.flush();
                children(element, out, depth);
                out.flush();
            }
        }
        "figure" | "figcaption" | "article" | "main" | "header" | "footer"
        | "nav" | "dl" | "dd" | "dt" | "li" | "center" | "body" | "html"
        | "tbody" | "thead" => {
            out.flush();
            children(element, out, depth);
            out.flush();
        }
        _ => {
            let inlines = inline(element, depth);
            out.inlines.extend(inlines);
        }
    }
}

fn blocks_at(element: ElementRef<'_>, depth: usize) -> Vec<Block> {
    let mut out = Out::default();
    children(element, &mut out, depth + 1);
    out.flush();
    out.blocks
}

fn callout(tone: &str, body: Vec<Block>) -> Block {
    Block::Container(Container {
        id: None,
        name: tone.into(),
        arg: String::new(),
        attrs: Attributes::default(),
        sections: vec![body],
    })
}

fn inline_children(element: ElementRef<'_>, depth: usize) -> Vec<Inline> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for child in element.children() {
        match child.value() {
            Node::Text(text) => push_text(&mut out, text),
            Node::Element(_) => {
                if let Some(child) = ElementRef::wrap(child) {
                    out.extend(inline(child, depth + 1));
                }
            }
            _ => {}
        }
    }
    out
}

fn inline(element: ElementRef<'_>, depth: usize) -> Vec<Inline> {
    let name = element.value().name();
    let wrap = |content: Vec<Inline>, f: fn(Vec<Inline>) -> Inline| {
        if content.is_empty() {
            Vec::new()
        } else {
            vec![f(content)]
        }
    };
    match name {
        "script" | "style" | "noscript" | "template" => Vec::new(),
        "strong" | "b" => wrap(inline_children(element, depth), |content| {
            Inline::Strong { content }
        }),
        "em" | "i" | "cite" => {
            wrap(inline_children(element, depth), |content| {
                Inline::Emphasis { content }
            })
        }
        "s" | "del" | "strike" => {
            wrap(inline_children(element, depth), |content| {
                Inline::Strikethrough { content }
            })
        }
        "mark" => wrap(inline_children(element, depth), |content| {
            Inline::Highlight { content }
        }),
        "sub" => wrap(inline_children(element, depth), |content| {
            Inline::Subscript { content }
        }),
        "sup" => wrap(inline_children(element, depth), |content| {
            Inline::Superscript { content }
        }),
        "code" | "tt" | "kbd" | "samp" => {
            let code: String = element.text().collect();
            if code.is_empty() {
                Vec::new()
            } else {
                vec![Inline::Code { code }]
            }
        }
        "br" => vec![Inline::HardBreak],
        "img" => image(element).into_iter().collect(),
        "a" => {
            let content = inline_children(element, depth);
            match element.value().attr("href") {
                Some(href)
                    if !href.trim().is_empty()
                        && !href.trim().starts_with("javascript:") =>
                {
                    vec![Inline::Link {
                        url: href.trim().to_owned(),
                        title: String::new(),
                        content,
                    }]
                }
                _ => content,
            }
        }
        _ => inline_children(element, depth),
    }
}

fn image(element: ElementRef<'_>) -> Option<Inline> {
    let alt = element.value().attr("alt").unwrap_or_default().to_owned();
    if has_class(element, "emoticon") {
        return Some(Inline::Text { text: alt });
    }
    let src = element
        .value()
        .attr("data-image-src")
        .or_else(|| element.value().attr("src"))?
        .trim();
    if src.is_empty() || src.starts_with("data:") {
        return None;
    }
    Some(Inline::Image {
        url: src.to_owned(),
        title: String::new(),
        alt: if alt.is_empty() {
            Vec::new()
        } else {
            vec![Inline::Text { text: alt }]
        },
    })
}

fn table(element: ElementRef<'_>, depth: usize) -> Option<Table> {
    let rows: Vec<(bool, Vec<Vec<Inline>>)> = element
        .select(&row_selector())
        // Skip rows of tables nested in cells.
        .filter(|tr| {
            tr.ancestors()
                .filter_map(ElementRef::wrap)
                .find(|a| a.value().name() == "table")
                .is_some_and(|t| t.id() == element.id())
        })
        .map(|tr| {
            let cells: Vec<ElementRef<'_>> = tr
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
                .collect();
            let header = !cells.is_empty()
                && cells.iter().all(|c| c.value().name() == "th");
            (header, cells.into_iter().map(|c| cell(c, depth)).collect())
        })
        .filter(|(_, cells): &(bool, Vec<Vec<Inline>>)| !cells.is_empty())
        .collect();
    let width = rows.iter().map(|(_, cells)| cells.len()).max()?;
    let mut rows = rows.into_iter();
    let (header, rest): (Vec<Vec<Inline>>, Vec<Vec<Vec<Inline>>>) =
        match rows.next() {
            Some((true, cells)) => (cells, rows.map(|(_, c)| c).collect()),
            Some((false, cells)) => (
                vec![Vec::new(); width],
                std::iter::once(cells).chain(rows.map(|(_, c)| c)).collect(),
            ),
            None => return None,
        };
    let pad = |mut cells: Vec<Vec<Inline>>| {
        cells.resize(width, Vec::new());
        cells
    };
    Some(Table {
        id: None,
        alignments: vec![Alignment::None; width],
        header: pad(header),
        rows: rest.into_iter().map(pad).collect(),
    })
}

/// A table cell's content as inlines: paragraphs and list items joined by
/// line breaks, since GFM cells hold one line.
fn cell(element: ElementRef<'_>, depth: usize) -> Vec<Inline> {
    let blocks = blocks_at(element, depth);
    let mut out = Vec::new();
    flatten_blocks(&blocks, &mut out);
    tidy(out)
}

fn flatten_blocks(blocks: &[Block], out: &mut Vec<Inline>) {
    for block in blocks {
        if !out.is_empty() {
            out.push(Inline::HardBreak);
        }
        match block {
            Block::Paragraph { content, .. }
            | Block::Heading { content, .. } => {
                out.extend(content.iter().cloned());
            }
            Block::List(list) => {
                for (i, item) in list.items.iter().enumerate() {
                    if i > 0 {
                        out.push(Inline::HardBreak);
                    }
                    out.push(Inline::Text {
                        text: list.start.map_or_else(
                            || "• ".to_owned(),
                            |s| format!("{}. ", s + i as u64),
                        ),
                    });
                    let mut inner = Vec::new();
                    flatten_blocks(&item.blocks, &mut inner);
                    out.extend(inner);
                }
            }
            Block::Fence(fence) => out.push(Inline::Code {
                code: fence.literal.trim_end().replace('\n', " "),
            }),
            Block::BlockQuote { blocks, .. } => flatten_blocks(blocks, out),
            Block::Container(container) => {
                for section in &container.sections {
                    flatten_blocks(section, out);
                }
            }
            Block::Table(table) => {
                for row in std::iter::once(&table.header).chain(&table.rows) {
                    for cell in row {
                        out.extend(cell.iter().cloned());
                        out.push(Inline::Text { text: " ".into() });
                    }
                }
            }
            Block::Math { literal, .. } => out.push(Inline::Code {
                code: literal.clone(),
            }),
            Block::ThematicBreak => {}
        }
    }
    if matches!(out.last(), Some(Inline::HardBreak)) {
        let _ = out.pop();
    }
}

fn push_text(out: &mut Vec<Inline>, text: &str) {
    let collapsed = collapse_keep_edges(text);
    if collapsed.is_empty() {
        return;
    }
    if let Some(Inline::Text { text: last }) = out.last_mut() {
        if last.ends_with(' ') && collapsed.starts_with(' ') {
            last.push_str(collapsed.trim_start());
        } else {
            last.push_str(&collapsed);
        }
    } else {
        out.push(Inline::Text { text: collapsed });
    }
}

/// Collapses runs of whitespace to one space, keeping a leading or
/// trailing space when there was whitespace there.
fn collapse_keep_edges(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

fn collapse(text: &str) -> String {
    collapse_keep_edges(text).trim().to_owned()
}

/// Trims a paragraph's edges, merges adjacent text and drops whitespace
/// that would become meaningful Markdown (leading spaces, trailing breaks).
fn tidy(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut merged: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (merged.last_mut(), inline) {
            (Some(Inline::Text { text: last }), Inline::Text { text }) => {
                if last.ends_with(' ') && text.starts_with(' ') {
                    last.push_str(text.trim_start());
                } else {
                    last.push_str(&text);
                }
            }
            (Some(Inline::HardBreak), Inline::Text { text }) => {
                let text = text.trim_start().to_owned();
                if !text.is_empty() {
                    merged.push(Inline::Text { text });
                }
            }
            (_, inline) => merged.push(inline),
        }
    }
    while matches!(merged.last(), Some(Inline::HardBreak)) {
        let _ = merged.pop();
    }
    while matches!(merged.first(), Some(Inline::HardBreak)) {
        let _ = merged.remove(0);
    }
    if let Some(Inline::Text { text }) = merged.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(Inline::Text { text }) = merged.last_mut() {
        *text = text.trim_end().to_owned();
    }
    merged.retain(|i| !matches!(i, Inline::Text { text } if text.is_empty()));
    merged
}

macro_rules! selector {
    ($name:ident, $css:literal) => {
        fn $name() -> scraper::Selector {
            static SELECTOR: std::sync::OnceLock<scraper::Selector> =
                std::sync::OnceLock::new();
            SELECTOR
                .get_or_init(|| {
                    scraper::Selector::parse($css)
                        .unwrap_or_else(|_| unreachable!("valid selector"))
                })
                .clone()
        }
    };
}

selector!(code_selector, "code");
selector!(body_selector, ".confluence-information-macro-body");
selector!(expand_title_selector, ".expand-control-text");
selector!(expand_body_selector, ".expand-content");
selector!(row_selector, "tr");

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::*;

    fn convert(html: &str) -> String {
        let doc = Html::parse_document(html);
        let body = doc
            .select(&scraper::Selector::parse("body").unwrap())
            .next()
            .unwrap();
        vellum_format::serialize(&vellum_format::Document {
            front_matter: None,
            blocks: blocks(body),
        })
    }

    #[test]
    fn common_html_maps_to_markdown() {
        let md = convert(
            r#"<body><h2>Rollout</h2><p>Ship <strong>behind</strong> a <a href="Flags_12.html">flag</a>
            and   <code>retry</code>.</p><ul><li>One</li><li>Two<ul><li>Nested</li></ul></li></ul>
            <ol start="3"><li>Third</li></ol><pre>let x = 1;</pre><hr/><blockquote><p>Quote</p></blockquote></body>"#,
        );
        assert!(md.contains("## Rollout"), "{md}");
        assert!(
            md.contains("Ship **behind** a [flag](Flags_12.html) and `retry`."),
            "{md}"
        );
        assert!(md.contains("- Two\n  - Nested"), "{md}");
        assert!(md.contains("3. Third"), "{md}");
        assert!(md.contains("let x = 1;"), "{md}");
        assert!(md.contains("> Quote"), "{md}");
    }

    #[test]
    fn confluence_macros_become_callouts_details_and_fences() {
        let md = convert(
            r#"<body><div class="confluence-information-macro confluence-information-macro-warning">
            <span class="confluence-information-macro-icon"></span>
            <div class="confluence-information-macro-body"><p>Do not restart.</p></div></div>
            <div class="expand-container"><div class="expand-control"><span class="expand-control-text">More</span></div>
            <div class="expand-content"><p>Hidden</p></div></div>
            <div class="code panel"><div class="codeContent"><pre class="syntaxhighlighter-pre" data-syntaxhighlighter-params="brush: py; gutter: false">print(1)</pre></div></div></body>"#,
        );
        assert!(md.contains("::: crit\n\nDo not restart.\n\n:::"), "{md}");
        assert!(md.contains("::: details More\n\nHidden\n\n:::"), "{md}");
        assert!(md.contains("```python\nprint(1)\n```"), "{md}");
    }

    #[test]
    fn tables_keep_header_and_flatten_cells() {
        let md = convert(
            "<body><table><tbody><tr><th>Key</th><th>Value</th></tr><tr><td><p>\
             a</p><p>b</p></td><td>c | d</td></tr></tbody></table></body>",
        );
        assert!(md.contains("| Key | Value |"), "{md}");
        assert!(md.contains("c \\| d"), "{md}");
    }

    #[test]
    fn hostile_markup_is_dropped() {
        let md = convert(
            "<body><script>alert(1)</script><p onclick=\"x\">Hi \
            <a href=\"javascript:alert(1)\">there</a></p></body>",
        );
        assert!(!md.contains("alert"), "{md}");
        assert!(md.contains("Hi there"), "{md}");
    }
}
