// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Vellum Markdown document model.
//!
//! The AST is owned, serde-serialisable and in canonical form: two inputs
//! that mean the same thing parse to equal trees (indented code becomes a
//! fence, `GitHub` alerts become callout containers, raw HTML becomes text,
//! adjacent text runs are merged). [`crate::serialize()`] writes it back as
//! canonical Markdown, and parsing that output yields the same tree.

use serde::Serialize;
use vellum_kernel::BlockId;

use crate::front_matter::FrontMatter;

/// A parsed Vellum Markdown file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Document {
    /// YAML front matter, when the file starts with `---`.
    pub front_matter: Option<FrontMatter>,
    /// Top-level blocks, in order.
    pub blocks: Vec<Block>,
}

/// Pandoc-style attributes: `{#id .class key="value"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Attributes {
    /// `#id`; the last one wins.
    pub id: Option<String>,
    /// `.class` entries, in order.
    pub classes: Vec<String>,
    /// `key="value"` pairs, in order.
    pub pairs: Vec<(String, String)>,
}

impl Attributes {
    /// True when no id, class or pair is set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.id.is_none() && self.classes.is_empty() && self.pairs.is_empty()
    }
}

/// A block-level node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    /// A paragraph, optionally ending in a `^blk-xxxxxxxx` block ID.
    Paragraph {
        /// Inline content.
        content: Vec<Inline>,
        /// Block ID written after the last line.
        id: Option<BlockId>,
    },
    /// An ATX heading (`#` to `######`). Setext headings are read as ATX.
    Heading {
        /// Level 1 to 6.
        level: u8,
        /// Inline content.
        content: Vec<Inline>,
        /// Trailing `{#id .class}` attributes.
        attrs: Attributes,
        /// Block ID written after the heading text.
        id: Option<BlockId>,
    },
    /// A horizontal rule.
    ThematicBreak,
    /// A `>` block quote.
    BlockQuote {
        /// Quoted blocks.
        blocks: Vec<Self>,
        /// Block ID, written `^blk-…` on its own line after the quote.
        id: Option<BlockId>,
    },
    /// A bullet, ordered or task list.
    List(List),
    /// A fenced block: source code, or a data fence such as `chart` or
    /// `mermaid` (see [`FenceKind`]). Indented code is read as a fence.
    Fence(Fence),
    /// Display math between `$$` lines (also read from a `math` fence).
    Math {
        /// `TeX` source, without the delimiters.
        literal: String,
        /// Block ID, written after the opening `$$`.
        id: Option<BlockId>,
    },
    /// A GFM table.
    Table(Table),
    /// A `:::` container such as a callout, `columns` or `tabs`.
    Container(Container),
}

/// A list and its items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct List {
    /// `None` for a bullet list, `Some(start)` for an ordered list.
    pub start: Option<u64>,
    /// Tight lists have no blank lines between items.
    pub tight: bool,
    /// The items.
    pub items: Vec<ListItem>,
}

/// One list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListItem {
    /// `Some(done)` for a task item (`[ ]` / `[x]`).
    pub task: Option<bool>,
    /// A task item's trailing `{…}` span (D82), written at the end of its
    /// first paragraph, before the block ID. Always empty for other items.
    pub attrs: Attributes,
    /// The item's blocks.
    pub blocks: Vec<Block>,
}

/// A fenced block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fence {
    /// First word of the info string (`rust`, `chart`, `mermaid`); may be empty.
    pub lang: String,
    /// The rest of the info string (`sortable bar=Revenue`), trimmed.
    pub args: String,
    /// Trailing `{#id .class}` attributes.
    pub attrs: Attributes,
    /// Block ID, the last word of the info string before the attributes.
    pub id: Option<BlockId>,
    /// The body, each line ending in `\n`.
    pub literal: String,
}

impl Fence {
    /// What the fence holds, from its language word.
    #[must_use]
    pub fn kind(&self) -> FenceKind {
        FenceKind::from_lang(&self.lang)
    }
}

/// The data fences of dialect v1 (brief §4); anything else is code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceKind {
    /// Source code or plain preformatted text; the readable fallback.
    Code,
    /// `chart`
    Chart,
    /// `timeline`
    Timeline,
    /// `roadmap`
    Roadmap,
    /// `risk-matrix`
    RiskMatrix,
    /// `board`
    Board,
    /// `table` (CSV with options)
    Table,
    /// `architecture`
    Architecture,
    /// `mermaid`, `d2`, `plantuml`, `graphviz`, `excalidraw`, `drawio`
    Diagram,
    /// `calculator`
    Calculator,
    /// `image`
    Image,
    /// `collection` (an embedded collection view)
    Collection,
}

impl FenceKind {
    /// Classifies a fence language word. Unknown words are [`FenceKind::Code`].
    #[must_use]
    pub fn from_lang(lang: &str) -> Self {
        match lang {
            "chart" => Self::Chart,
            "timeline" => Self::Timeline,
            "roadmap" => Self::Roadmap,
            "risk-matrix" => Self::RiskMatrix,
            "board" => Self::Board,
            "table" => Self::Table,
            "architecture" => Self::Architecture,
            "mermaid" | "d2" | "plantuml" | "graphviz" | "excalidraw" | "drawio" => Self::Diagram,
            "calculator" => Self::Calculator,
            "image" => Self::Image,
            "collection" => Self::Collection,
            _ => Self::Code,
        }
    }
}

/// Column alignment in a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// No colon in the delimiter row.
    None,
    /// `:--`
    Left,
    /// `:-:`
    Center,
    /// `--:`
    Right,
}

/// A GFM table. Every row has as many cells as the header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    /// One entry per column.
    pub alignments: Vec<Alignment>,
    /// Header cells.
    pub header: Vec<Vec<Inline>>,
    /// Body rows of cells.
    pub rows: Vec<Vec<Vec<Inline>>>,
    /// Block ID, written `^blk-…` on its own line after the table.
    pub id: Option<BlockId>,
}

/// A `:::` container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Container {
    /// The word after the colons (`info`, `columns`, `details`); may be empty.
    pub name: String,
    /// Text after the name (`details` title, `link` URL), trimmed.
    pub arg: String,
    /// Trailing `{#id .class}` attributes.
    pub attrs: Attributes,
    /// Block ID, the last word of the opening line before the attributes.
    pub id: Option<BlockId>,
    /// The body. `columns` has one entry per column, split by `---` lines;
    /// every other container has exactly one entry.
    pub sections: Vec<Vec<Block>>,
}

impl Container {
    /// What the container is, from its name.
    #[must_use]
    pub fn kind(&self) -> ContainerKind {
        ContainerKind::from_name(&self.name)
    }
}

/// The containers of dialect v1 (brief §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerKind {
    /// `info warn good crit tip note`
    Callout,
    /// `details <title>`
    Details,
    /// `columns` (2–4, split by `---`)
    Columns,
    /// `tabs` (`##` per tab)
    Tabs,
    /// `toc`
    Toc,
    /// `stats` (value · label · delta)
    Stats,
    /// `facts` (key: value)
    Facts,
    /// `cards`
    Cards,
    /// `steps`
    Steps,
    /// `progress`
    Progress,
    /// `link <url>`
    Link,
    /// `embed <url>` (sandboxed)
    Embed,
    /// `pages`
    Pages,
    /// Anything else: its body is shown as ordinary blocks.
    Unknown,
}

impl ContainerKind {
    /// Classifies a container name. Unknown names are [`ContainerKind::Unknown`].
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "info" | "warn" | "good" | "crit" | "tip" | "note" => Self::Callout,
            "details" => Self::Details,
            "columns" => Self::Columns,
            "tabs" => Self::Tabs,
            "toc" => Self::Toc,
            "stats" => Self::Stats,
            "facts" => Self::Facts,
            "cards" => Self::Cards,
            "steps" => Self::Steps,
            "progress" => Self::Progress,
            "link" => Self::Link,
            "embed" => Self::Embed,
            "pages" => Self::Pages,
            _ => Self::Unknown,
        }
    }
}

/// An inline node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    /// Literal text (never empty; adjacent runs are merged).
    Text {
        /// The text, unescaped.
        text: String,
    },
    /// `` `code` ``
    Code {
        /// The code, unescaped.
        code: String,
    },
    /// A line break inside a paragraph.
    SoftBreak,
    /// A hard line break, written `\` at the end of the line.
    HardBreak,
    /// `*emphasis*`
    Emphasis {
        /// Content.
        content: Vec<Self>,
    },
    /// `**strong**`
    Strong {
        /// Content.
        content: Vec<Self>,
    },
    /// `~~strikethrough~~`
    Strikethrough {
        /// Content.
        content: Vec<Self>,
    },
    /// `==highlight==`
    Highlight {
        /// Content.
        content: Vec<Self>,
    },
    /// `~subscript~`
    Subscript {
        /// Content.
        content: Vec<Self>,
    },
    /// `^superscript^`
    Superscript {
        /// Content.
        content: Vec<Self>,
    },
    /// `[text](url "title")`, or a bare autolink when the text is the URL.
    Link {
        /// Destination.
        url: String,
        /// Title; may be empty.
        title: String,
        /// Link text.
        content: Vec<Self>,
    },
    /// `![alt](url "title")`
    Image {
        /// Source.
        url: String,
        /// Title; may be empty.
        title: String,
        /// Alt text.
        alt: Vec<Self>,
    },
    /// `[[Target]]` or `[[Target|label]]`.
    WikiLink {
        /// Page title or `vellum://` link.
        target: String,
        /// Label after the pipe, when different from the target.
        label: Option<String>,
    },
    /// `@name`, a person or group.
    Mention {
        /// The handle, without `@`.
        handle: String,
    },
    /// `ENG-142`, a candidate issue key; resolved against teams when rendered.
    IssueKey {
        /// The key as written.
        key: String,
    },
    /// A calendar date (`2026-11-24`) or `@today`.
    Date {
        /// `YYYY-MM-DD`, or `today`.
        date: String,
    },
}
