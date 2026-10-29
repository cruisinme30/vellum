// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The production [`MarkdownCodec`]: Vellum Markdown v1 through
//! `vellum-format`'s parser and canonical serializer.
//!
//! This module also fixes the editor's node schema. `y-prosemirror` stores
//! node and mark type names verbatim, so the web editor's `ProseMirror`
//! schema must use exactly these names and attributes (ADR 0009, D55).
//!
//! | Markdown | Node | Attributes | Content |
//! | --- | --- | --- | --- |
//! | paragraph | `paragraph` | `blockId` | inline |
//! | `#` heading | `heading` | `level`, `blockId`, `anchor`, `classes`, `pairs` | inline |
//! | `---` | `horizontal_rule` | | |
//! | `>` quote | `blockquote` | | blocks |
//! | `-` list | `bullet_list` | `tight` | `list_item` |
//! | `1.` list | `ordered_list` | `start`, `tight` | `list_item` |
//! | list item | `list_item` | `checked` (task items only) | blocks |
//! | fence | `code_block` | `language`, `args`, `anchor`, `classes`, `pairs` | text |
//! | `$$` math | `math_block` | | text |
//! | table | `table` | | `table_row` |
//! | table row | `table_row` | | `table_header` (first row) or `table_cell` |
//! | table cell | `table_header`, `table_cell` | `align` | one `paragraph` |
//! | `:::` container | `container` | `name`, `arg`, `anchor`, `classes`, `pairs` | `container_section` (one per `columns` column) |
//! | container body | `container_section` | | blocks |
//!
//! Inline nodes: `image` (`src`, `title`, `alt`), `hard_break`, `wiki_link`
//! (`target`, `label`), `mention` (`handle`), `issue_key` (`key`) and `date`
//! (`date`). Each may carry a `marks` attribute, a map of the marks around
//! it. Marks: `em`, `strong`, `strike`, `highlight`, `subscript`,
//! `superscript`, `code` and `link` (`href`, `title`). A soft line break is
//! a `\n` in the text. Code block and math text has no final newline.
//!
//! Front matter fields are the keys of the `meta` map. Front matter that is
//! not a YAML mapping is kept verbatim under [`RAW_FRONT_MATTER`]. Yjs
//! maps are unordered, so [`FRONT_MATTER_ORDER`] lists the file's key
//! order; keys missing from it are written after, sorted.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Number as JsonNumber, Value};
use vellum_format::{
    Alignment, Attributes, Block, Container, Document, Fence, FrontMatter, Inline, List, ListItem,
    Table,
};
use vellum_kernel::{BlockId, Result};
use yrs::{Any, Number};

use crate::ports::MarkdownCodec;
use crate::tree::{DocTree, Element, Node, TextRun};

/// The `meta` key that holds front matter which is not a YAML mapping.
pub const RAW_FRONT_MATTER: &str = "$raw";

/// The `meta` key that lists front matter keys in file order.
pub const FRONT_MATTER_ORDER: &str = "$order";

/// Marks that wrap content, outermost first when two cover the same span.
const WRAPPING_MARKS: [&str; 7] = [
    "link",
    "em",
    "strong",
    "strike",
    "highlight",
    "subscript",
    "superscript",
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
        Block::Paragraph { content, id } => {
            with_block_id(Element::new("paragraph", inlines_to_nodes(content)), *id)
        }
        Block::Heading {
            level,
            content,
            attrs,
            id,
        } => {
            let element = Element::new("heading", inlines_to_nodes(content))
                .with_attr("level", i64::from(*level));
            with_attributes(with_block_id(element, *id), attrs)
        }
        Block::ThematicBreak => Element::new("horizontal_rule", Vec::new()),
        Block::BlockQuote { blocks } => Element::new("blockquote", blocks_to_nodes(blocks)),
        Block::List(list) => list_to_element(list),
        Block::Fence(fence) => {
            let element = Element::new("code_block", literal_to_nodes(&fence.literal))
                .with_attr("language", fence.lang.clone())
                .with_attr("args", fence.args.clone());
            with_attributes(element, &fence.attrs)
        }
        Block::Math { literal } => Element::new("math_block", literal_to_nodes(literal)),
        Block::Table(table) => table_to_element(table),
        Block::Container(container) => {
            let sections = container
                .sections
                .iter()
                .map(|blocks| {
                    Node::Element(Element::new("container_section", blocks_to_nodes(blocks)))
                })
                .collect();
            let element = Element::new("container", sections)
                .with_attr("name", container.name.clone())
                .with_attr("arg", container.arg.clone());
            with_attributes(element, &container.attrs)
        }
    }
}

fn list_to_element(list: &List) -> Element {
    let items = list
        .items
        .iter()
        .map(|item| {
            let element = Element::new("list_item", blocks_to_nodes(&item.blocks));
            Node::Element(match item.task {
                Some(done) => element.with_attr("checked", done),
                None => element,
            })
        })
        .collect();
    let element = match list.start {
        Some(start) => Element::new("ordered_list", items).with_attr("start", u64_to_any(start)),
        None => Element::new("bullet_list", items),
    };
    element.with_attr("tight", list.tight)
}

fn table_to_element(table: &Table) -> Element {
    let row = |cells: &[Vec<Inline>], cell_name: &str| {
        let cells = cells
            .iter()
            .enumerate()
            .map(|(column, inlines)| {
                let paragraph = Element::new("paragraph", inlines_to_nodes(inlines));
                let cell = Element::new(cell_name, vec![Node::Element(paragraph)]);
                let align = match table.alignments.get(column) {
                    Some(Alignment::Left) => Some("left"),
                    Some(Alignment::Center) => Some("center"),
                    Some(Alignment::Right) => Some("right"),
                    Some(Alignment::None) | None => None,
                };
                Node::Element(match align {
                    Some(align) => cell.with_attr("align", align),
                    None => cell,
                })
            })
            .collect();
        Node::Element(Element::new("table_row", cells))
    };
    let mut rows = vec![row(&table.header, "table_header")];
    rows.extend(table.rows.iter().map(|cells| row(cells, "table_cell")));
    Element::new("table", rows)
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

fn with_attributes(mut element: Element, attrs: &Attributes) -> Element {
    if let Some(anchor) = &attrs.id {
        element = element.with_attr("anchor", anchor.clone());
    }
    if !attrs.classes.is_empty() {
        let classes: Vec<Any> = attrs.classes.iter().map(|c| Any::from(c.clone())).collect();
        element = element.with_attr("classes", Any::Array(classes.into()));
    }
    if !attrs.pairs.is_empty() {
        let pairs: Vec<Any> = attrs
            .pairs
            .iter()
            .map(|(key, value)| {
                Any::Array(vec![Any::from(key.clone()), Any::from(value.clone())].into())
            })
            .collect();
        element = element.with_attr("pairs", Any::Array(pairs.into()));
    }
    element
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
    let with = |name: &str, value: Any| {
        let mut marks = marks.clone();
        let _previous = marks.insert(name.to_owned(), value);
        marks
    };
    let empty = || Any::Map(Arc::default());
    for inline in inlines {
        match inline {
            Inline::Text { text } => out.push(Piece::Text(text.clone(), marks.clone())),
            Inline::Code { code } => out.push(Piece::Text(code.clone(), with("code", empty()))),
            Inline::SoftBreak => out.push(Piece::Text("\n".to_owned(), marks.clone())),
            Inline::HardBreak => out.push(Piece::Atom(
                Element::new("hard_break", Vec::new()),
                marks.clone(),
            )),
            Inline::Emphasis { content } => flatten(content, &with("em", empty()), out),
            Inline::Strong { content } => flatten(content, &with("strong", empty()), out),
            Inline::Strikethrough { content } => flatten(content, &with("strike", empty()), out),
            Inline::Highlight { content } => flatten(content, &with("highlight", empty()), out),
            Inline::Subscript { content } => flatten(content, &with("subscript", empty()), out),
            Inline::Superscript { content } => {
                flatten(content, &with("superscript", empty()), out);
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
                flatten(content, &with("link", Any::Map(link.into())), out);
            }
            Inline::Image { url, title, alt } => out.push(Piece::Atom(
                Element::new("image", Vec::new())
                    .with_attr("src", url.clone())
                    .with_attr("title", title.clone())
                    .with_attr("alt", vellum_format::plain_text(alt)),
                marks.clone(),
            )),
            Inline::WikiLink { target, label } => {
                let element =
                    Element::new("wiki_link", Vec::new()).with_attr("target", target.clone());
                let element = match label {
                    Some(label) => element.with_attr("label", label.clone()),
                    None => element,
                };
                out.push(Piece::Atom(element, marks.clone()));
            }
            Inline::Mention { handle } => out.push(Piece::Atom(
                Element::new("mention", Vec::new()).with_attr("handle", handle.clone()),
                marks.clone(),
            )),
            Inline::IssueKey { key } => out.push(Piece::Atom(
                Element::new("issue_key", Vec::new()).with_attr("key", key.clone()),
                marks.clone(),
            )),
            Inline::Date { date } => out.push(Piece::Atom(
                Element::new("date", Vec::new()).with_attr("date", date.clone()),
                marks.clone(),
            )),
        }
    }
}

fn marks_to_any(marks: &Marks) -> Any {
    let map: HashMap<String, Any> = marks
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Any::Map(map.into())
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

fn is_inline(name: &str) -> bool {
    matches!(
        name,
        "image" | "hard_break" | "wiki_link" | "mention" | "issue_key" | "date"
    )
}

fn element_to_blocks(element: &Element, out: &mut Vec<Block>) {
    let attrs = &element.attrs;
    match element.name.as_str() {
        "paragraph" => out.push(Block::Paragraph {
            content: nodes_to_inlines(&element.children),
            id: block_id(attrs),
        }),
        "heading" => {
            let level = attr_u64(attrs, "level").unwrap_or(1).clamp(1, 6);
            out.push(Block::Heading {
                level: u8::try_from(level).unwrap_or(1),
                content: nodes_to_inlines(&element.children),
                attrs: attributes(attrs),
                id: block_id(attrs),
            });
        }
        "horizontal_rule" => out.push(Block::ThematicBreak),
        "blockquote" => out.push(Block::BlockQuote {
            blocks: nodes_to_blocks(&element.children),
        }),
        "bullet_list" | "ordered_list" => out.push(Block::List(element_to_list(element))),
        "code_block" => out.push(Block::Fence(Fence {
            lang: attr_str(attrs, "language").unwrap_or_default().to_owned(),
            args: attr_str(attrs, "args").unwrap_or_default().to_owned(),
            attrs: attributes(attrs),
            literal: nodes_to_literal(&element.children),
        })),
        "math_block" => out.push(Block::Math {
            literal: nodes_to_literal(&element.children),
        }),
        "table" => out.extend(element_to_table(element).map(Block::Table)),
        "container" => out.push(Block::Container(element_to_container(element))),
        // Unknown or misplaced nodes keep their content.
        _ => out.extend(nodes_to_blocks(&element.children)),
    }
}

fn element_to_list(element: &Element) -> List {
    let start =
        (element.name == "ordered_list").then(|| attr_u64(&element.attrs, "start").unwrap_or(1));
    let tight = attr_bool(&element.attrs, "tight").unwrap_or(true);
    let items = element
        .children
        .iter()
        .map(|child| match child {
            Node::Element(item) if item.name == "list_item" => ListItem {
                task: attr_bool(&item.attrs, "checked"),
                blocks: nodes_to_blocks(&item.children),
            },
            other => ListItem {
                task: None,
                blocks: nodes_to_blocks(std::slice::from_ref(other)),
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
    let rows: Vec<&Element> = element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(row) if row.name == "table_row" => Some(row),
            _ => None,
        })
        .collect();
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

fn element_to_container(element: &Element) -> Container {
    let sections: Vec<&Element> = element
        .children
        .iter()
        .filter_map(|child| match child {
            Node::Element(section) if section.name == "container_section" => Some(section),
            _ => None,
        })
        .collect();
    let sections = if sections.is_empty() {
        vec![nodes_to_blocks(&element.children)]
    } else {
        sections
            .iter()
            .map(|section| nodes_to_blocks(&section.children))
            .collect()
    };
    Container {
        name: attr_str(&element.attrs, "name")
            .unwrap_or_default()
            .to_owned(),
        arg: attr_str(&element.attrs, "arg")
            .unwrap_or_default()
            .to_owned(),
        attrs: attributes(&element.attrs),
        sections,
    }
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
                let marks = match element.attrs.get("marks") {
                    Some(Any::Map(map)) => map
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    _ => Marks::new(),
                };
                out.push(Piece::Atom(element.clone(), marks));
            }
            // A block inside inline content: keep its text.
            Node::Element(element) => collect_pieces(&element.children, out),
        }
    }
}

/// Rebuilds nested inline nodes from flat marks. At each point the mark
/// that stays on longest opens first, so `**a *b***` comes back as
/// strong around (text, emphasis).
fn build_inlines(pieces: &[Piece], open: &Marks) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(piece) = pieces.get(index) {
        let next = WRAPPING_MARKS
            .iter()
            .enumerate()
            .filter_map(|(rank, name)| {
                let value = piece.marks().get(*name)?;
                if open.get(*name) == Some(value) {
                    return None;
                }
                let span = pieces
                    .get(index..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|p| p.marks().get(*name) == Some(value))
                    .count();
                Some((span, rank, *name, value))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        match next {
            None => {
                leaf(piece, &mut out);
                index += 1;
            }
            Some((span, _, name, value)) => {
                let mut inner = open.clone();
                let _previous = inner.insert(name.to_owned(), value.clone());
                let end = index + span;
                let content = build_inlines(pieces.get(index..end).unwrap_or_default(), &inner);
                out.push(wrap(name, value, content));
                index = end;
            }
        }
    }
    merge_text(out)
}

fn leaf(piece: &Piece, out: &mut Vec<Inline>) {
    match piece {
        Piece::Text(text, marks) if marks.contains_key("code") => {
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

fn atom(element: &Element) -> Option<Inline> {
    let attrs = &element.attrs;
    let text = |key: &str| attr_str(attrs, key).unwrap_or_default().to_owned();
    Some(match element.name.as_str() {
        "hard_break" => Inline::HardBreak,
        "image" => {
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
        "wiki_link" => Inline::WikiLink {
            target: text("target"),
            label: attr_str(attrs, "label").map(str::to_owned),
        },
        "mention" => Inline::Mention {
            handle: text("handle"),
        },
        "issue_key" => Inline::IssueKey { key: text("key") },
        "date" => Inline::Date { date: text("date") },
        _ => return None,
    })
}

fn wrap(name: &str, value: &Any, content: Vec<Inline>) -> Inline {
    match name {
        "strong" => Inline::Strong { content },
        "em" => Inline::Emphasis { content },
        "strike" => Inline::Strikethrough { content },
        "highlight" => Inline::Highlight { content },
        "subscript" => Inline::Subscript { content },
        "superscript" => Inline::Superscript { content },
        _ => {
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
    let strings = |value: &Any| match value {
        Any::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Any::String(text) => Some(text.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Attributes {
        id: attr_str(attrs, "anchor").map(str::to_owned),
        classes: attrs.get("classes").map(strings).unwrap_or_default(),
        pairs: match attrs.get("pairs") {
            Some(Any::Array(pairs)) => pairs
                .iter()
                .filter_map(|pair| match strings(pair).as_slice() {
                    [key, value] => Some((key.clone(), value.clone())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
    }
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
