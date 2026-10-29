// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The production [`MarkdownCodec`]: Vellum Markdown v1 through
//! `vellum-format`'s parser and canonical serializer, in the editor's
//! block model.
//!
//! Node and mark names come from [`vellum_contracts::blocks`], generated
//! from `schema/blocks.json` (ADR 0010), so the server and the web editor
//! cannot drift. The mapping, Markdown to node:
//!
//! - paragraph, heading, blockquote, `---` (`horizontalRule`), fences
//!   (`codeBlock`, or `dataFence` for data languages), `$$` (`mathBlock`),
//!   tables (`table` > `tableRow` > `tableHeader` / `tableCell` > one
//!   `paragraph`), lists (`taskList` when every item of a bullet list is a
//!   task, else `bulletList` or `orderedList`; task items are `taskItem`)
//!   and a paragraph holding only an image (`image`).
//! - Containers become their typed node when the body fits it (`callout`,
//!   `details`, `columns` > `column`, `tabs` > `tab` split at `##`, `toc`,
//!   `stats`, `facts`, `progress`, `cards`, `pages`, `steps`, `linkCard`,
//!   `embed`); anything else is a generic `container` node.
//! - Inline: text with the marks `bold`, `italic`, `strike`, `code`,
//!   `highlight`, `subscript`, `superscript` and `link`; a soft break is a
//!   `\n` in the text; `hardBreak`, `mention`, `wikiLink`, `issueKey`,
//!   `date`, and `inlineImage` for an image inside text. Marks around an
//!   inline node are its `marks` attribute, as `y-prosemirror` stores them.
//!
//! Dialect facts the schema has no attribute for yet are kept in extra
//! element attributes ([`ATTRIBUTES`] for a `{...}` span, `args` on
//! `codeBlock`, `tight` on lists, `title` on `callout`), so a round trip
//! never loses them. Code and math text has no final newline.
//!
//! Front matter fields are the keys of the `meta` map. Front matter that is
//! not a YAML mapping is kept verbatim under [`RAW_FRONT_MATTER`]. Yjs
//! maps are unordered, so [`FRONT_MATTER_ORDER`] lists the file's key
//! order; keys missing from it are written after, sorted.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Number as JsonNumber, Value};
use vellum_contracts::blocks::{MarkType, NodeType};
use vellum_format::{
    Alignment, Attributes, Block, Container, ContainerKind, Document, Fence, FenceKind,
    FrontMatter, Inline, List, ListItem, Table,
};
use vellum_kernel::{BlockId, Result};
use yrs::{Any, Number};

use crate::ports::MarkdownCodec;
use crate::tree::{DocTree, Element, Node, TextRun};

/// The `meta` key that holds front matter which is not a YAML mapping.
pub const RAW_FRONT_MATTER: &str = "$raw";

/// The `meta` key that lists front matter keys in file order.
pub const FRONT_MATTER_ORDER: &str = "$order";

/// The verbatim `{#id .class key="value"}` span of a heading, fence or
/// container, as `vellum_format::attrs` writes it.
pub const ATTRIBUTES: &str = "attributes";

/// Marks that wrap content, outermost first when two cover the same span.
const WRAPPING_MARKS: [MarkType; 7] = [
    MarkType::Link,
    MarkType::Italic,
    MarkType::Bold,
    MarkType::Strike,
    MarkType::Highlight,
    MarkType::Subscript,
    MarkType::Superscript,
];

/// [`MarkdownCodec`] over `vellum_format::parse` and `vellum_format::serialize`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FormatCodec;

impl MarkdownCodec for FormatCodec {
    fn to_markdown(&self, doc: &DocTree) -> Result<String> {
        let first = vellum_format::serialize(&to_document(doc));
        // A tree built in the editor may not be canonical (front matter key
        // order, neighbouring text); one parse and serialize makes it so.
        Ok(vellum_format::serialize(
            &vellum_format::parse(&first).document,
        ))
    }

    fn parse_markdown(&self, markdown: &str) -> Result<DocTree> {
        Ok(from_document(&vellum_format::parse(markdown).document))
    }
}

fn node(node: NodeType, children: Vec<Node>) -> Element {
    Element::new(node.name(), children)
}

// ---------------------------------------------------------------------------
// Markdown AST → editor tree
// ---------------------------------------------------------------------------

/// Converts a parsed document to the editor's tree.
#[must_use]
pub fn from_document(document: &Document) -> DocTree {
    let meta = match &document.front_matter {
        None => BTreeMap::new(),
        Some(FrontMatter::Fields(fields)) => {
            let mut meta: BTreeMap<String, Any> = fields
                .iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect();
            if fields.len() > 1 {
                let order: Vec<Any> = fields.keys().map(|key| Any::from(key.clone())).collect();
                let _previous =
                    meta.insert(FRONT_MATTER_ORDER.to_owned(), Any::Array(order.into()));
            }
            meta
        }
        Some(FrontMatter::Raw(raw)) => {
            BTreeMap::from([(RAW_FRONT_MATTER.to_owned(), Any::from(raw.clone()))])
        }
    };
    DocTree {
        meta,
        content: blocks_to_nodes(&document.blocks),
    }
}

fn blocks_to_nodes(blocks: &[Block]) -> Vec<Node> {
    blocks
        .iter()
        .map(|block| Node::Element(block_to_element(block)))
        .collect()
}

fn block_to_element(block: &Block) -> Element {
    match block {
        Block::Paragraph { content, id } => match (content.as_slice(), id) {
            ([Inline::Image { url, title, alt }], _) => with_block_id(
                node(NodeType::Image, Vec::new())
                    .with_attr("src", url.clone())
                    .with_attr("alt", vellum_format::plain_text(alt))
                    .with_attr("title", title.clone()),
                *id,
            ),
            _ => with_block_id(node(NodeType::Paragraph, inlines_to_nodes(content)), *id),
        },
        Block::Heading {
            level,
            content,
            attrs,
            id,
        } => {
            let element = node(NodeType::Heading, inlines_to_nodes(content))
                .with_attr("level", i64::from(*level));
            with_attributes(with_block_id(element, *id), attrs)
        }
        Block::ThematicBreak => node(NodeType::HorizontalRule, Vec::new()),
        Block::BlockQuote { blocks, id } => {
            with_block_id(node(NodeType::Blockquote, blocks_to_nodes(blocks)), *id)
        }
        Block::List(list) => list_to_element(list),
        Block::Fence(fence) => {
            let element = if fence.kind() == FenceKind::Code {
                let element = node(NodeType::CodeBlock, literal_to_nodes(&fence.literal))
                    .with_attr("language", fence.lang.clone());
                if fence.args.is_empty() {
                    element
                } else {
                    element.with_attr("args", fence.args.clone())
                }
            } else {
                node(NodeType::DataFence, literal_to_nodes(&fence.literal))
                    .with_attr("lang", fence.lang.clone())
                    .with_attr("args", fence.args.clone())
            };
            with_attributes(with_block_id(element, fence.id), &fence.attrs)
        }
        Block::Math { literal, id } => {
            with_block_id(node(NodeType::MathBlock, literal_to_nodes(literal)), *id)
        }
        Block::Table(table) => table_to_element(table),
        Block::Container(container) => {
            let element = typed_container(container).unwrap_or_else(|| {
                let section = container
                    .sections
                    .first()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                node(NodeType::Container, blocks_to_nodes(section))
                    .with_attr("name", container.name.clone())
                    .with_attr("arg", container.arg.clone())
            });
            with_attributes(with_block_id(element, container.id), &container.attrs)
        }
    }
}

/// The typed node for a container, when its body and argument fit one.
fn typed_container(container: &Container) -> Option<Element> {
    let kind = container.kind();
    let body: &[Block] = match container.sections.as_slice() {
        [body] => body,
        _ if kind == ContainerKind::Columns => &[],
        _ => return None,
    };
    let no_arg = container.arg.is_empty();
    let only_list = |ordered: bool| match body {
        [Block::List(list)]
            if list.start.is_some() == ordered && list.items.iter().all(|i| i.task.is_none()) =>
        {
            Some(blocks_to_nodes(body))
        }
        _ => None,
    };
    Some(match kind {
        ContainerKind::Callout => {
            let kind = vellum_contracts::blocks::CalloutKind::from_name(&container.name)?;
            let element =
                node(NodeType::Callout, blocks_to_nodes(body)).with_attr("kind", kind.name());
            if no_arg {
                element
            } else {
                element.with_attr("title", container.arg.clone())
            }
        }
        ContainerKind::Details => node(NodeType::Details, blocks_to_nodes(body))
            .with_attr("summary", container.arg.clone()),
        ContainerKind::Columns if no_arg && container.sections.len() >= 2 => {
            let columns = container
                .sections
                .iter()
                .map(|blocks| Node::Element(node(NodeType::Column, blocks_to_nodes(blocks))))
                .collect();
            node(NodeType::Columns, columns)
        }
        ContainerKind::Tabs if no_arg => node(NodeType::Tabs, split_tabs(body)?),
        ContainerKind::Toc if no_arg && body.is_empty() => node(NodeType::Toc, Vec::new()),
        ContainerKind::Stats if no_arg => node(NodeType::Stats, only_list(false)?),
        ContainerKind::Facts if no_arg => node(NodeType::Facts, only_list(false)?),
        ContainerKind::Progress if no_arg => node(NodeType::Progress, only_list(false)?),
        ContainerKind::Cards if no_arg => node(NodeType::Cards, only_list(false)?),
        ContainerKind::Steps if no_arg => node(NodeType::Steps, only_list(true)?),
        ContainerKind::Pages if no_arg && body.is_empty() => node(NodeType::Pages, Vec::new()),
        ContainerKind::Pages if no_arg => node(NodeType::Pages, only_list(false)?),
        ContainerKind::Link => {
            // The body is one line holding the card's title.
            let title = match body {
                [] => String::new(),
                [Block::Paragraph { content, id: None }] => match content.as_slice() {
                    [Inline::Text { text }] => text.clone(),
                    _ => return None,
                },
                _ => return None,
            };
            node(NodeType::LinkCard, Vec::new())
                .with_attr("url", container.arg.clone())
                .with_attr("title", title)
        }
        ContainerKind::Embed if body.is_empty() => {
            node(NodeType::Embed, Vec::new()).with_attr("url", container.arg.clone())
        }
        _ => return None,
    })
}

/// Splits a `tabs` body at its level-2 headings. `None` when the body does
/// not start with one, or a heading holds more than plain text.
fn split_tabs(body: &[Block]) -> Option<Vec<Node>> {
    let mut tabs: Vec<(String, Vec<Node>)> = Vec::new();
    for block in body {
        match block {
            Block::Heading {
                level: 2,
                content,
                attrs,
                id: None,
            } if attrs.is_empty() => {
                let [Inline::Text { text }] = content.as_slice() else {
                    return None;
                };
                tabs.push((text.clone(), Vec::new()));
            }
            other => tabs
                .last_mut()?
                .1
                .push(Node::Element(block_to_element(other))),
        }
    }
    if tabs.is_empty() {
        return None;
    }
    Some(
        tabs.into_iter()
            .map(|(title, blocks)| {
                Node::Element(node(NodeType::Tab, blocks).with_attr("title", title))
            })
            .collect(),
    )
}

fn list_to_element(list: &List) -> Element {
    let all_tasks = !list.items.is_empty() && list.items.iter().all(|item| item.task.is_some());
    let task_list = all_tasks && list.start.is_none();
    let items = list
        .items
        .iter()
        .map(|item| {
            let blocks = blocks_to_nodes(&item.blocks);
            Node::Element(item.task.map_or_else(
                || node(NodeType::ListItem, blocks.clone()),
                |done| {
                    let task = node(NodeType::TaskItem, blocks.clone()).with_attr("checked", done);
                    with_attributes(task, &item.attrs)
                },
            ))
        })
        .collect();
    let element = match list.start {
        _ if task_list => node(NodeType::TaskList, items),
        Some(start) => node(NodeType::OrderedList, items).with_attr("start", u64_to_any(start)),
        None => node(NodeType::BulletList, items),
    };
    if list.tight {
        element
    } else {
        element.with_attr("tight", false)
    }
}

fn table_to_element(table: &Table) -> Element {
    let row = |cells: &[Vec<Inline>], cell_kind: NodeType| {
        let cells = cells
            .iter()
            .enumerate()
            .map(|(column, inlines)| {
                let paragraph = node(NodeType::Paragraph, inlines_to_nodes(inlines));
                let align = match table.alignments.get(column) {
                    Some(Alignment::Left) => "left",
                    Some(Alignment::Center) => "center",
                    Some(Alignment::Right) => "right",
                    Some(Alignment::None) | None => "none",
                };
                Node::Element(
                    node(cell_kind, vec![Node::Element(paragraph)]).with_attr("align", align),
                )
            })
            .collect();
        Node::Element(node(NodeType::TableRow, cells))
    };
    let mut rows = vec![row(&table.header, NodeType::TableHeader)];
    rows.extend(
        table
            .rows
            .iter()
            .map(|cells| row(cells, NodeType::TableCell)),
    );
    with_block_id(node(NodeType::Table, rows), table.id)
}

/// Block text without its final newline. An empty literal has no text
/// node, so a body of one blank line (an empty text node) survives.
fn literal_to_nodes(literal: &str) -> Vec<Node> {
    match literal.strip_suffix('\n') {
        None if literal.is_empty() => Vec::new(),
        Some("") => vec![Node::Text(Vec::new())],
        text => vec![Node::Text(vec![TextRun::plain(text.unwrap_or(literal))])],
    }
}

fn with_block_id(element: Element, id: Option<BlockId>) -> Element {
    match id {
        Some(id) => element.with_attr("blockId", id.to_string()),
        None => element,
    }
}

fn with_attributes(element: Element, attrs: &Attributes) -> Element {
    if attrs.is_empty() {
        element
    } else {
        element.with_attr(ATTRIBUTES, vellum_format::attrs::to_string(attrs))
    }
}

type Marks = BTreeMap<String, Any>;

/// Flat inline content: text runs and inline nodes, each with its marks.
enum Piece {
    Text(String, Marks),
    Atom(Element, Marks),
}

impl Piece {
    const fn marks(&self) -> &Marks {
        match self {
            Self::Text(_, marks) | Self::Atom(_, marks) => marks,
        }
    }
}

fn inlines_to_nodes(inlines: &[Inline]) -> Vec<Node> {
    let mut pieces = Vec::new();
    flatten(inlines, &Marks::new(), &mut pieces);
    let mut nodes = Vec::new();
    let mut runs: Vec<TextRun> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Text(text, marks) => runs.push(TextRun { text, marks }),
            Piece::Atom(mut element, marks) => {
                if !runs.is_empty() {
                    nodes.push(Node::Text(std::mem::take(&mut runs)));
                }
                if !marks.is_empty() {
                    element = element.with_attr("marks", marks_to_any(&marks));
                }
                nodes.push(Node::Element(element));
            }
        }
    }
    if !runs.is_empty() {
        nodes.push(Node::Text(runs));
    }
    nodes
        .into_iter()
        .map(|node| match node {
            Node::Text(_) => node.normalized(),
            Node::Element(_) => node,
        })
        .collect()
}

fn flatten(inlines: &[Inline], marks: &Marks, out: &mut Vec<Piece>) {
    let with = |mark: MarkType, value: Any| {
        let mut marks = marks.clone();
        let _previous = marks.insert(mark.name().to_owned(), value);
        marks
    };
    let empty = || Any::Map(Arc::default());
    let atom = |element: Element| Piece::Atom(element, marks.clone());
    for inline in inlines {
        match inline {
            Inline::Text { text } => out.push(Piece::Text(text.clone(), marks.clone())),
            Inline::Code { code } => {
                out.push(Piece::Text(code.clone(), with(MarkType::Code, empty())));
            }
            Inline::SoftBreak => out.push(Piece::Text("\n".to_owned(), marks.clone())),
            Inline::HardBreak => out.push(atom(node(NodeType::HardBreak, Vec::new()))),
            Inline::Emphasis { content } => flatten(content, &with(MarkType::Italic, empty()), out),
            Inline::Strong { content } => flatten(content, &with(MarkType::Bold, empty()), out),
            Inline::Strikethrough { content } => {
                flatten(content, &with(MarkType::Strike, empty()), out);
            }
            Inline::Highlight { content } => {
                flatten(content, &with(MarkType::Highlight, empty()), out);
            }
            Inline::Subscript { content } => {
                flatten(content, &with(MarkType::Subscript, empty()), out);
            }
            Inline::Superscript { content } => {
                flatten(content, &with(MarkType::Superscript, empty()), out);
            }
            Inline::Link {
                url,
                title,
                content,
            } => {
                let link = HashMap::from([
                    ("href".to_owned(), Any::from(url.clone())),
                    ("title".to_owned(), Any::from(title.clone())),
                ]);
                flatten(content, &with(MarkType::Link, Any::Map(link.into())), out);
            }
            Inline::Image { url, title, alt } => out.push(atom(
                node(NodeType::InlineImage, Vec::new())
                    .with_attr("src", url.clone())
                    .with_attr("alt", vellum_format::plain_text(alt))
                    .with_attr("title", title.clone()),
            )),
            Inline::WikiLink { target, label } => out.push(atom(
                node(NodeType::WikiLink, Vec::new())
                    .with_attr("target", target.clone())
                    .with_attr("label", label.clone().unwrap_or_default()),
            )),
            Inline::Mention { handle } => out.push(atom(
                node(NodeType::Mention, Vec::new()).with_attr("handle", handle.clone()),
            )),
            Inline::IssueKey { key } => out.push(atom(
                node(NodeType::IssueKey, Vec::new()).with_attr("key", key.clone()),
            )),
            Inline::Date { date } => out.push(atom(
                node(NodeType::Date, Vec::new()).with_attr("date", date.clone()),
            )),
        }
    }
}

/// Marks in the `y-prosemirror` form for inline nodes: `[{type, attrs}]`.
fn marks_to_any(marks: &Marks) -> Any {
    let marks: Vec<Any> = marks
        .iter()
        .map(|(mark, attrs)| {
            let mut entry = HashMap::from([("type".to_owned(), Any::from(mark.clone()))]);
            if matches!(attrs, Any::Map(map) if !map.is_empty()) {
                let _previous = entry.insert("attrs".to_owned(), attrs.clone());
            }
            Any::Map(entry.into())
        })
        .collect();
    Any::Array(marks.into())
}

fn marks_from_any(value: Option<&Any>) -> Marks {
    let Some(Any::Array(marks)) = value else {
        return Marks::new();
    };
    marks
        .iter()
        .filter_map(|mark| {
            let Any::Map(mark) = mark else {
                return None;
            };
            let Some(Any::String(kind)) = mark.get("type") else {
                return None;
            };
            let attrs = mark
                .get("attrs")
                .cloned()
                .unwrap_or_else(|| Any::Map(Arc::default()));
            Some((kind.to_string(), attrs))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Editor tree → Markdown AST
// ---------------------------------------------------------------------------

/// Converts the editor's tree to a document. Never fails: unknown nodes
/// keep their content and bad attributes fall back to defaults, so a
/// version can always be written.
#[must_use]
pub fn to_document(doc: &DocTree) -> Document {
    let front_matter = match doc.meta.get(RAW_FRONT_MATTER) {
        Some(Any::String(raw)) if doc.meta.len() == 1 => Some(FrontMatter::Raw(raw.to_string())),
        _ if doc.meta.is_empty() => None,
        _ => Some(FrontMatter::Fields(fields(&doc.meta))),
    };
    Document {
        front_matter,
        blocks: nodes_to_blocks(&doc.content),
    }
}

/// Front matter fields in file order, then any new keys sorted.
fn fields(meta: &BTreeMap<String, Any>) -> Map<String, Value> {
    let is_field = |key: &str| key != RAW_FRONT_MATTER && key != FRONT_MATTER_ORDER;
    let mut out = Map::new();
    if let Some(Any::Array(order)) = meta.get(FRONT_MATTER_ORDER) {
        for key in order.iter() {
            if let Any::String(key) = key
                && is_field(key)
                && let Some(value) = meta.get(key.as_ref())
            {
                let _previous = out.insert(key.to_string(), any_to_json(value));
            }
        }
    }
    for (key, value) in meta {
        if is_field(key) && !out.contains_key(key) {
            let _previous = out.insert(key.clone(), any_to_json(value));
        }
    }
    out
}

fn is_inline(name: &str) -> bool {
    NodeType::from_name(name).is_some_and(|kind| kind.is_inline() && kind != NodeType::Text)
}

fn nodes_to_blocks(nodes: &[Node]) -> Vec<Block> {
    let mut blocks = Vec::new();
    // Inline content directly inside a block container becomes a paragraph.
    let mut loose: Vec<Node> = Vec::new();
    let flush = |loose: &mut Vec<Node>, blocks: &mut Vec<Block>| {
        if !loose.is_empty() {
            blocks.push(Block::Paragraph {
                content: nodes_to_inlines(loose),
                id: None,
            });
            loose.clear();
        }
    };
    for node in nodes {
        match node {
            Node::Element(element) if !is_inline(&element.name) => {
                flush(&mut loose, &mut blocks);
                element_to_blocks(element, &mut blocks);
            }
            Node::Element(_) | Node::Text(_) => loose.push(node.clone()),
        }
    }
    flush(&mut loose, &mut blocks);
    blocks
}

fn element_to_blocks(element: &Element, out: &mut Vec<Block>) {
    let attrs = &element.attrs;
    let children = &element.children;
    let text = |key: &str| attr_str(attrs, key).unwrap_or_default().to_owned();
    if let Some(container) = element_to_container(element) {
        out.push(Block::Container(container));
        return;
    }
    let Some(kind) = NodeType::from_name(&element.name) else {
        // Unknown nodes keep their content.
        out.extend(nodes_to_blocks(children));
        return;
    };
    match kind {
        NodeType::Paragraph => out.push(Block::Paragraph {
            content: nodes_to_inlines(children),
            id: block_id(attrs),
        }),
        NodeType::Heading => {
            let level = attr_u64(attrs, "level").unwrap_or(1).clamp(1, 6);
            out.push(Block::Heading {
                level: u8::try_from(level).unwrap_or(1),
                content: nodes_to_inlines(children),
                attrs: attributes(attrs),
                id: block_id(attrs),
            });
        }
        NodeType::HorizontalRule => out.push(Block::ThematicBreak),
        NodeType::Blockquote => out.push(Block::BlockQuote {
            blocks: nodes_to_blocks(children),
            id: block_id(attrs),
        }),
        NodeType::BulletList | NodeType::OrderedList | NodeType::TaskList => {
            out.push(Block::List(element_to_list(element, kind)));
        }
        NodeType::CodeBlock => out.push(Block::Fence(Fence {
            lang: text("language"),
            args: text("args"),
            attrs: attributes(attrs),
            id: block_id(attrs),
            literal: nodes_to_literal(children),
        })),
        NodeType::DataFence => out.push(Block::Fence(Fence {
            lang: text("lang"),
            args: text("args"),
            attrs: attributes(attrs),
            id: block_id(attrs),
            literal: nodes_to_literal(children),
        })),
        NodeType::MathBlock => out.push(Block::Math {
            literal: nodes_to_literal(children),
            id: block_id(attrs),
        }),
        NodeType::Table => out.extend(element_to_table(element).map(Block::Table)),
        NodeType::Image => out.push(Block::Paragraph {
            content: vec![image(attrs)],
            id: block_id(attrs),
        }),
        // Misplaced nodes (list items, rows, cells, columns, tabs, inline
        // nodes reached here) keep their content.
        _ => out.extend(nodes_to_blocks(children)),
    }
}

/// The `:::` container a node stands for, if it is one.
fn element_to_container(element: &Element) -> Option<Container> {
    let attrs = &element.attrs;
    let children = &element.children;
    let text = |key: &str| attr_str(attrs, key).unwrap_or_default().to_owned();
    let container = |name: String, arg: String, sections: Vec<Vec<Block>>| Container {
        name,
        arg,
        attrs: attributes(attrs),
        id: block_id(attrs),
        sections,
    };
    let named =
        |name: &str, arg: String| container(name.to_owned(), arg, vec![nodes_to_blocks(children)]);
    match NodeType::from_name(&element.name)? {
        NodeType::Container => Some(named(&text("name"), text("arg"))),
        NodeType::Callout => {
            let name = attr_str(attrs, "kind").unwrap_or("info");
            Some(named(name, text("title")))
        }
        NodeType::Details => Some(named("details", text("summary"))),
        NodeType::Columns => {
            let sections = children
                .iter()
                .map(|child| match child {
                    Node::Element(column) => nodes_to_blocks(&column.children),
                    Node::Text(_) => nodes_to_blocks(std::slice::from_ref(child)),
                })
                .collect();
            Some(container("columns".to_owned(), String::new(), sections))
        }
        NodeType::Tabs => {
            let mut body = Vec::new();
            for child in children {
                let Node::Element(tab) = child else {
                    continue;
                };
                body.push(Block::Heading {
                    level: 2,
                    content: vec![Inline::Text {
                        text: attr_str(&tab.attrs, "title").unwrap_or("Tab").to_owned(),
                    }],
                    attrs: Attributes::default(),
                    id: None,
                });
                body.extend(nodes_to_blocks(&tab.children));
            }
            Some(container("tabs".to_owned(), String::new(), vec![body]))
        }
        NodeType::Toc => Some(named("toc", String::new())),
        NodeType::Stats => Some(named("stats", String::new())),
        NodeType::Facts => Some(named("facts", String::new())),
        NodeType::Progress => Some(named("progress", String::new())),
        NodeType::Cards => Some(named("cards", String::new())),
        NodeType::Steps => Some(named("steps", String::new())),
        NodeType::Pages => Some(named("pages", String::new())),
        NodeType::LinkCard => {
            let title = text("title");
            let body = if title.is_empty() {
                Vec::new()
            } else {
                vec![Block::Paragraph {
                    content: vec![Inline::Text { text: title }],
                    id: None,
                }]
            };
            Some(container("link".to_owned(), text("url"), vec![body]))
        }
        NodeType::Embed => Some(named("embed", text("url"))),
        _ => None,
    }
}

fn element_to_list(element: &Element, kind: NodeType) -> List {
    let start =
        (kind == NodeType::OrderedList).then(|| attr_u64(&element.attrs, "start").unwrap_or(1));
    let tight = attr_bool(&element.attrs, "tight").unwrap_or(true);
    let items = element
        .children
        .iter()
        .map(|child| match child {
            Node::Element(item) => {
                let task = (NodeType::from_name(&item.name) == Some(NodeType::TaskItem))
                    .then(|| attr_bool(&item.attrs, "checked").unwrap_or(false));
                ListItem {
                    task,
                    attrs: if task.is_some() {
                        attributes(&item.attrs)
                    } else {
                        Attributes::default()
                    },
                    blocks: nodes_to_blocks(&item.children),
                }
            }
            Node::Text(_) => ListItem {
                task: None,
                attrs: Attributes::default(),
                blocks: nodes_to_blocks(std::slice::from_ref(child)),
            },
        })
        .collect();
    List {
        start,
        tight,
        items,
    }
}

fn cells(row: &Element) -> Vec<&Element> {
    row.children
        .iter()
        .filter_map(|child| match child {
            Node::Element(cell) => Some(cell),
            Node::Text(_) => None,
        })
        .collect()
}

fn element_to_table(element: &Element) -> Option<Table> {
    let rows = cells(element);
    let (header_row, body_rows) = rows.split_first()?;
    let header_cells = cells(header_row);
    if header_cells.is_empty() {
        return None;
    }
    let alignments = header_cells
        .iter()
        .map(|cell| match attr_str(&cell.attrs, "align") {
            Some("left") => Alignment::Left,
            Some("center") => Alignment::Center,
            Some("right") => Alignment::Right,
            _ => Alignment::None,
        })
        .collect();
    let width = header_cells.len();
    let header = header_cells.iter().map(|cell| cell_inlines(cell)).collect();
    let rows = body_rows
        .iter()
        .map(|row| {
            let mut out: Vec<Vec<Inline>> =
                cells(row).iter().map(|cell| cell_inlines(cell)).collect();
            out.resize(width, Vec::new());
            out
        })
        .collect();
    Some(Table {
        alignments,
        header,
        rows,
        id: block_id(&element.attrs),
    })
}

/// A cell's inline content. A table cell is one line, so several
/// paragraphs are joined with a space.
fn cell_inlines(cell: &Element) -> Vec<Inline> {
    let mut out = Vec::new();
    for block in nodes_to_blocks(&cell.children) {
        let (Block::Paragraph { content, .. } | Block::Heading { content, .. }) = block else {
            continue;
        };
        if !out.is_empty() && !content.is_empty() {
            out.push(Inline::Text {
                text: " ".to_owned(),
            });
        }
        out.extend(content);
    }
    merge_text(out)
}

fn nodes_to_literal(nodes: &[Node]) -> String {
    let mut text = String::new();
    collect_text(nodes, &mut text);
    if !nodes.is_empty() {
        text.push('\n');
    }
    text
}

fn collect_text(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(runs) => runs.iter().for_each(|run| out.push_str(&run.text)),
            Node::Element(element) => collect_text(&element.children, out),
        }
    }
}

fn nodes_to_inlines(nodes: &[Node]) -> Vec<Inline> {
    let mut pieces = Vec::new();
    collect_pieces(nodes, &mut pieces);
    build_inlines(&pieces, &Marks::new())
}

fn collect_pieces(nodes: &[Node], out: &mut Vec<Piece>) {
    for node in nodes {
        match node {
            Node::Text(runs) => out.extend(
                runs.iter()
                    .filter(|run| !run.text.is_empty())
                    .map(|run| Piece::Text(run.text.clone(), run.marks.clone())),
            ),
            Node::Element(element) if is_inline(&element.name) => {
                let marks = marks_from_any(element.attrs.get("marks"));
                out.push(Piece::Atom(element.clone(), marks));
            }
            // A block inside inline content: keep its text.
            Node::Element(element) => collect_pieces(&element.children, out),
        }
    }
}

/// Rebuilds nested inline nodes from flat marks. At each point the mark
/// that stays on longest opens first, so `**a *b***` comes back as
/// bold around (text, italic).
fn build_inlines(pieces: &[Piece], open: &Marks) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(piece) = pieces.get(index) {
        let next = WRAPPING_MARKS
            .iter()
            .enumerate()
            .filter_map(|(rank, mark)| {
                let name = mark.name();
                let value = piece.marks().get(name)?;
                if open.get(name) == Some(value) {
                    return None;
                }
                let span = pieces
                    .get(index..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|p| p.marks().get(name) == Some(value))
                    .count();
                Some((span, rank, *mark, value))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        match next {
            None => {
                leaf(piece, &mut out);
                index += 1;
            }
            Some((span, _, mark, value)) => {
                let mut inner = open.clone();
                let _previous = inner.insert(mark.name().to_owned(), value.clone());
                let end = index + span;
                let content = build_inlines(pieces.get(index..end).unwrap_or_default(), &inner);
                out.push(wrap(mark, value, content));
                index = end;
            }
        }
    }
    merge_text(out)
}

fn leaf(piece: &Piece, out: &mut Vec<Inline>) {
    match piece {
        Piece::Text(text, marks) if marks.contains_key(MarkType::Code.name()) => {
            out.push(Inline::Code { code: text.clone() });
        }
        Piece::Text(text, _) => {
            for (n, line) in text.split('\n').enumerate() {
                if n > 0 {
                    out.push(Inline::SoftBreak);
                }
                if !line.is_empty() {
                    out.push(Inline::Text {
                        text: line.to_owned(),
                    });
                }
            }
        }
        Piece::Atom(element, _) => out.extend(atom(element)),
    }
}

fn image(attrs: &BTreeMap<String, Any>) -> Inline {
    let text = |key: &str| attr_str(attrs, key).unwrap_or_default().to_owned();
    let alt = text("alt");
    Inline::Image {
        url: text("src"),
        title: text("title"),
        alt: if alt.is_empty() {
            Vec::new()
        } else {
            vec![Inline::Text { text: alt }]
        },
    }
}

fn atom(element: &Element) -> Option<Inline> {
    let attrs = &element.attrs;
    let text = |key: &str| attr_str(attrs, key).unwrap_or_default().to_owned();
    Some(match NodeType::from_name(&element.name)? {
        NodeType::HardBreak => Inline::HardBreak,
        NodeType::InlineImage => image(attrs),
        NodeType::WikiLink => Inline::WikiLink {
            target: text("target"),
            label: attr_str(attrs, "label")
                .filter(|label| !label.is_empty())
                .map(str::to_owned),
        },
        NodeType::Mention => Inline::Mention {
            handle: text("handle"),
        },
        NodeType::IssueKey => Inline::IssueKey { key: text("key") },
        NodeType::Date => Inline::Date { date: text("date") },
        // The dialect has no inline math yet; keep the source readable.
        NodeType::MathInline => Inline::Text {
            text: format!("${}$", text("literal")),
        },
        _ => return None,
    })
}

fn wrap(mark: MarkType, value: &Any, content: Vec<Inline>) -> Inline {
    match mark {
        MarkType::Bold => Inline::Strong { content },
        MarkType::Italic => Inline::Emphasis { content },
        MarkType::Strike => Inline::Strikethrough { content },
        MarkType::Highlight => Inline::Highlight { content },
        MarkType::Subscript => Inline::Subscript { content },
        MarkType::Superscript => Inline::Superscript { content },
        MarkType::Link | MarkType::Code => {
            let field = |key: &str| match value {
                Any::Map(map) => match map.get(key) {
                    Some(Any::String(text)) => text.to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            Inline::Link {
                url: field("href"),
                title: field("title"),
                content,
            }
        }
    }
}

/// Joins neighbouring text nodes, as the parser does.
fn merge_text(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        if let (Some(Inline::Text { text: last }), Inline::Text { text }) =
            (out.last_mut(), &inline)
        {
            last.push_str(text);
            continue;
        }
        out.push(inline);
    }
    out
}

// ---------------------------------------------------------------------------
// Attributes and values
// ---------------------------------------------------------------------------

fn attr_str<'a>(attrs: &'a BTreeMap<String, Any>, key: &str) -> Option<&'a str> {
    match attrs.get(key) {
        Some(Any::String(text)) => Some(text),
        _ => None,
    }
}

fn attr_bool(attrs: &BTreeMap<String, Any>, key: &str) -> Option<bool> {
    match attrs.get(key) {
        Some(Any::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn attr_u64(attrs: &BTreeMap<String, Any>, key: &str) -> Option<u64> {
    match attrs.get(key)? {
        Any::Number(number) => number.as_i64().and_then(|value| u64::try_from(value).ok()),
        Any::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn block_id(attrs: &BTreeMap<String, Any>) -> Option<BlockId> {
    attr_str(attrs, "blockId").and_then(|id| id.parse().ok())
}

fn attributes(attrs: &BTreeMap<String, Any>) -> Attributes {
    attr_str(attrs, ATTRIBUTES)
        .and_then(vellum_format::attrs::parse)
        .unwrap_or_default()
}

fn u64_to_any(value: u64) -> Any {
    Number::try_from(value).map_or_else(|big| Any::from(big.to_string()), Any::Number)
}

/// Converts a front matter value. Maps lose their key order: Yjs maps are
/// unordered.
fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(value) => Any::Bool(*value),
        Value::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(int), _) => Any::Number(Number::Int(int)),
            (None, Some(int)) => u64_to_any(int),
            (None, None) => Any::Number(Number::Float(number.as_f64().unwrap_or_default())),
        },
        Value::String(text) => Any::from(text.clone()),
        Value::Array(items) => Any::Array(items.iter().map(json_to_any).collect::<Vec<_>>().into()),
        Value::Object(map) => Any::Map(
            map.iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect::<HashMap<_, _>>()
                .into(),
        ),
    }
}

/// Converts back. Nested map keys are sorted, so output is deterministic.
fn any_to_json(value: &Any) -> Value {
    match value {
        Any::Null | Any::Undefined | Any::Buffer(_) => Value::Null,
        Any::Bool(value) => Value::Bool(*value),
        Any::Number(Number::Int(int)) => Value::Number(int.to_owned().into()),
        Any::Number(Number::Float(float)) => {
            JsonNumber::from_f64(*float).map_or(Value::Null, Value::Number)
        }
        Any::String(text) => Value::String(text.to_string()),
        Any::Array(items) => Value::Array(items.iter().map(any_to_json).collect()),
        Any::Map(map) => {
            let sorted: BTreeMap<&String, &Any> = map.iter().collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.clone(), any_to_json(value)))
                    .collect(),
            )
        }
    }
}
