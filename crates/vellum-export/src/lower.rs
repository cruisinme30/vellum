// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Lowering: a [`Document`] becomes [`Node`]s for one export target.
//!
//! Lowering runs in three steps. A synchronous walk collects everything
//! the document needs from outside (figures, live data, images, issue
//! chips, page titles, people); those requests then run concurrently
//! through the [`Ports`]; and a second walk builds the nodes from the
//! answers. Nothing here fails: a block that cannot be shown as designed
//! falls back to readable text and adds an [`ExportWarning`].
//!
//! The mapping follows the PRD block catalogue's "In exports" column:
//! charts and diagrams are vector figures, roadmaps, timelines, boards and
//! risk matrices are drawn in print and become dated lists or tables in
//! office formats, toggles and tabs are expanded under their titles, and
//! live blocks are frozen at [`LowerOptions::as_of`].

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use futures::future::join_all;
use vellum_format::{
    Alignment, Block, Container, ContainerKind, Document, Fence, FenceKind, Inline, ListItem,
};

use crate::data;
use crate::error::ExportWarning;
use crate::flow::{
    Align, Asset, Bar, Card, Cell, Flow, Item, Layout, Media, Node, Span, Stat, Style, Table,
    TableStyle, Tone,
};
use crate::math;
use crate::ports::{FigureError, FigureSvg, IssueChip, PageRef, Ports, Snapshot};

/// Most images and figures one export may carry. The print backend gives
/// each a file slot, so the limit also bounds the typesetter's file table.
pub const MAX_ASSETS: usize = 2_000;

/// Largest image accepted, in bytes.
pub const MAX_ASSET_BYTES: usize = 40 * 1024 * 1024;

/// The kind of format being produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// PDF: figures for timelines, roadmaps, boards and risk matrices.
    Print,
    /// Word and Google Docs: those blocks become tables and lists.
    Office,
}

/// Where cross-references point inside one export.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refs {
    /// Document ID → (anchor, reference text such as `§2.3 Rollback`).
    pub docs: HashMap<String, (String, String)>,
    /// Block ID (`blk-…`) → (anchor, reference text).
    pub blocks: HashMap<String, (String, String)>,
}

/// Settings for lowering one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerOptions {
    /// The output family.
    pub target: Target,
    /// The date live blocks are frozen at, `YYYY-MM-DD`.
    pub as_of: String,
    /// Public base URL of the workspace (`https://vellum.northstar.dev`),
    /// for links to pages outside the export. Without it such links
    /// export as their text.
    pub base_url: Option<String>,
    /// Levels added to every heading (a book section at depth 2 adds 2).
    pub heading_offset: u8,
    /// Prefix that keeps anchors unique when several documents are
    /// exported together.
    pub anchor_prefix: String,
    /// Cross-reference targets in this export.
    pub refs: Refs,
}

impl LowerOptions {
    /// Options for a single document on `target`, frozen at `as_of`.
    pub fn new(target: Target, as_of: impl Into<String>) -> Self {
        Self {
            target,
            as_of: as_of.into(),
            base_url: None,
            heading_offset: 0,
            anchor_prefix: String::new(),
            refs: Refs::default(),
        }
    }
}

/// A lowered export with what could not be shown as designed.
#[derive(Debug, Clone, PartialEq)]
pub struct Lowered {
    /// The export model.
    pub flow: Flow,
    /// Blocks that fell back to readable text.
    pub warnings: Vec<ExportWarning>,
}

/// Lowers one document for `opts.target`.
pub async fn lower(doc: &Document, ports: &Ports, opts: &LowerOptions) -> Lowered {
    let mut session = Session::new(ports);
    let title = doc_title(doc);
    let blocks = session.lower(doc, opts).await;
    let mut lowered = session.finish(title, Layout::Document, &opts.as_of, blocks);
    lowered.flow.show_title = !matches!(
        doc.blocks.first(),
        Some(Block::Heading { level: 1, content, .. }) if inline_text(content) == lowered.flow.title
    );
    lowered
}

/// The title of a document: front matter `title`, else the first heading.
#[must_use]
pub fn doc_title(doc: &Document) -> String {
    if let Some(title) = doc.front_matter.as_ref().and_then(|fm| fm.title()) {
        return title.to_owned();
    }
    doc.blocks
        .iter()
        .find_map(|block| match block {
            Block::Heading { content, .. } => Some(inline_text(content)),
            _ => None,
        })
        .unwrap_or_default()
}

/// Plain text of AST inlines.
#[must_use]
pub fn inline_text(inlines: &[Inline]) -> String {
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
            | Inline::Link { content, .. } => out.push_str(&inline_text(content)),
            Inline::Image { alt, .. } => out.push_str(&inline_text(alt)),
            Inline::WikiLink { target, label } => out.push_str(label.as_deref().unwrap_or(target)),
            Inline::Mention { handle } => {
                out.push('@');
                out.push_str(handle);
            }
            Inline::IssueKey { key } => out.push_str(key),
            Inline::Date { date } => out.push_str(date),
        }
    }
    out
}

/// A URL-safe anchor from heading text: lower case, words joined by `-`.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        let _ = out.pop();
    }
    if out.is_empty() {
        "section".to_owned()
    } else {
        out
    }
}

/// Lowers several documents into one export, sharing images, figures and
/// warnings. Book export uses it; [`lower`] wraps it for one document.
pub struct Session<'p> {
    ports: &'p Ports,
    assets: Vec<Asset>,
    by_url: HashMap<String, Option<String>>,
    warnings: Vec<ExportWarning>,
    anchors: HashSet<String>,
}

impl<'p> Session<'p> {
    /// A new session using `ports`.
    #[must_use]
    pub fn new(ports: &'p Ports) -> Self {
        Self {
            ports,
            assets: Vec::new(),
            by_url: HashMap::new(),
            warnings: Vec::new(),
            anchors: HashSet::new(),
        }
    }

    /// Lowers `doc` with `opts`, resolving everything it needs first.
    pub async fn lower(&mut self, doc: &Document, opts: &LowerOptions) -> Vec<Node> {
        let mut wants = Wants::default();
        wants.blocks(&doc.blocks, opts.target);
        let answers = self.resolve(wants).await;
        let mut ctx = Ctx {
            opts,
            answers: &answers,
            session: self,
            anchor_seen: HashSet::new(),
        };
        ctx.blocks(&doc.blocks)
    }

    /// Adds a warning that is not tied to one block.
    pub fn warn(&mut self, warning: ExportWarning) {
        self.warnings.push(warning);
    }

    /// Finishes the export.
    #[must_use]
    pub fn finish(self, title: String, layout: Layout, as_of: &str, blocks: Vec<Node>) -> Lowered {
        Lowered {
            flow: Flow {
                title,
                as_of: as_of.to_owned(),
                layout,
                show_title: true,
                blocks,
                assets: self.assets,
            },
            warnings: self.warnings,
        }
    }

    async fn resolve(&mut self, wants: Wants) -> Answers {
        let ports = self.ports;
        let figures = join_all(wants.figures.into_iter().map(|fence| async move {
            let key = fence_key(&fence);
            let result = if data::arg(&fence, "source").is_some() {
                match ports.live.chart(&fence).await {
                    Ok(resolved) => ports.figures.render(&resolved).await.map(|svg| (svg, Some(resolved))),
                    Err(error) => Err(FigureError::Unavailable(error.0)),
                }
            } else {
                ports.figures.render(&fence).await.map(|svg| (svg, None))
            };
            (key, result)
        }));
        let collections = join_all(wants.collections.into_iter().map(|fence| async move {
            (fence_key(&fence), ports.live.collection(&fence).await)
        }));
        let images = join_all(
            wants
                .images
                .into_iter()
                .filter(|url| !self.by_url.contains_key(url))
                .map(|url| async move {
                    let result = ports.assets.fetch(&url).await;
                    (url, result)
                }),
        );
        let issues = join_all(
            wants
                .issues
                .into_iter()
                .map(|key| async move { (key.clone(), ports.live.issue(&key).await) }),
        );
        let pages = join_all(
            wants
                .pages
                .into_iter()
                .map(|t| async move { (t.clone(), ports.live.page(&t).await) }),
        );
        let people = join_all(
            wants
                .people
                .into_iter()
                .map(|h| async move { (h.clone(), ports.live.person(&h).await) }),
        );
        let (figures, collections, images, issues, pages, people) =
            futures::join!(figures, collections, images, issues, pages, people);

        let mut answers = Answers {
            figures: HashMap::new(),
            collections: collections.into_iter().collect(),
            issues: issues.into_iter().filter_map(|(k, v)| Some((k, v?))).collect(),
            pages: pages.into_iter().filter_map(|(k, v)| Some((k, v?))).collect(),
            people: people.into_iter().filter_map(|(k, v)| Some((k, v?))).collect(),
        };
        for (url, result) in images {
            let name = match result {
                Ok(bytes) => self.add_asset(&url, bytes),
                Err(error) => {
                    self.warnings
                        .push(ExportWarning::new(url.clone(), error.to_string()));
                    None
                }
            };
            let _ = self.by_url.insert(url, name);
        }
        for (key, result) in figures {
            let value = match result {
                Ok((svg, resolved)) => match self.add_figure(&svg) {
                    Some(asset) => Ok(FigureAnswer {
                        asset,
                        svg,
                        resolved,
                    }),
                    None => Err(FigureError::Unavailable("too many figures".into())),
                },
                Err(error) => Err(error),
            };
            let _ = answers.figures.insert(key, value);
        }
        answers
    }

    fn add_asset(&mut self, url: &str, bytes: Vec<u8>) -> Option<String> {
        if bytes.len() > MAX_ASSET_BYTES {
            self.warnings.push(ExportWarning::new(
                url,
                "the image is larger than 40 MB and was left out",
            ));
            return None;
        }
        let Some(media) = Media::sniff(&bytes) else {
            self.warnings.push(ExportWarning::new(
                url,
                "the file is not a PNG, JPEG, GIF or SVG image and was left out",
            ));
            return None;
        };
        if self.assets.len() >= MAX_ASSETS {
            self.warnings.push(ExportWarning::new(
                url,
                format!("the export already holds {MAX_ASSETS} images; this one was left out"),
            ));
            return None;
        }
        let (width, height) = image_size(media, &bytes);
        let name = format!("a{}.{}", self.assets.len(), media.extension());
        self.assets.push(Asset {
            name: name.clone(),
            media,
            bytes,
            width,
            height,
        });
        Some(name)
    }

    fn add_figure(&mut self, svg: &FigureSvg) -> Option<String> {
        if self.assets.len() >= MAX_ASSETS {
            return None;
        }
        let name = format!("a{}.svg", self.assets.len());
        self.assets.push(Asset {
            name: name.clone(),
            media: Media::Svg,
            bytes: svg.svg.clone().into_bytes(),
            width: px(svg.width),
            height: px(svg.height),
        });
        Some(name)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped first
fn px(value: f64) -> Option<u32> {
    (value.is_finite() && value > 0.0).then(|| value.min(100_000.0).round() as u32)
}

/// Width and height of a raster image from its header.
fn image_size(media: Media, bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    let be32 = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at + 4)?;
        Some(u32::from_be_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
    };
    let le16 = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at + 2)?;
        Some(u32::from(u16::from_le_bytes([*b.first()?, *b.get(1)?])))
    };
    match media {
        Media::Png => (be32(16), be32(20)),
        Media::Gif => (le16(6), le16(8)),
        Media::Jpeg => jpeg_size(bytes),
        Media::Svg => (None, None),
    }
}

fn jpeg_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    let mut at = 2;
    while let (Some(&marker_lead), Some(&marker)) = (bytes.get(at), bytes.get(at + 1)) {
        if marker_lead != 0xFF {
            break;
        }
        let len = bytes
            .get(at + 2..at + 4)
            .map(|b| usize::from(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)])))
            .unwrap_or(0);
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = bytes.get(at + 5..at + 7).map(|b| {
                u32::from(u16::from_be_bytes([
                    b.first().copied().unwrap_or(0),
                    b.get(1).copied().unwrap_or(0),
                ]))
            });
            let w = bytes.get(at + 7..at + 9).map(|b| {
                u32::from(u16::from_be_bytes([
                    b.first().copied().unwrap_or(0),
                    b.get(1).copied().unwrap_or(0),
                ]))
            });
            return (w, h);
        }
        if len < 2 {
            break;
        }
        at += 2 + len;
    }
    (None, None)
}

/// A stable key for a fence, to match answers to blocks.
fn fence_key(fence: &Fence) -> String {
    format!("{}\u{0}{}\u{0}{}", fence.lang, fence.args, fence.literal)
}

/// What a document needs from outside.
#[derive(Default)]
struct Wants {
    figures: Vec<Fence>,
    collections: Vec<Fence>,
    images: Vec<String>,
    issues: Vec<String>,
    pages: Vec<String>,
    people: Vec<String>,
    seen: HashSet<String>,
}

impl Wants {
    fn once(&mut self, key: String) -> bool {
        self.seen.insert(key)
    }

    fn blocks(&mut self, blocks: &[Block], target: Target) {
        for block in blocks {
            self.block(block, target);
        }
    }

    fn block(&mut self, block: &Block, target: Target) {
        match block {
            Block::Paragraph { content, .. } | Block::Heading { content, .. } => {
                self.inlines(content);
            }
            Block::BlockQuote { blocks } => self.blocks(blocks, target),
            Block::List(list) => {
                for item in &list.items {
                    self.blocks(&item.blocks, target);
                }
            }
            Block::Table(table) => {
                for cell in table.header.iter().chain(table.rows.iter().flatten()) {
                    self.inlines(cell);
                }
            }
            Block::Container(container) => {
                for section in &container.sections {
                    self.blocks(section, target);
                }
            }
            Block::Fence(fence) => self.fence(fence, target),
            Block::ThematicBreak | Block::Math { .. } => {}
        }
    }

    fn fence(&mut self, fence: &Fence, target: Target) {
        let wants_figure = match fence.kind() {
            FenceKind::Chart | FenceKind::Diagram | FenceKind::Architecture => true,
            FenceKind::Timeline | FenceKind::Roadmap | FenceKind::RiskMatrix | FenceKind::Board => {
                target == Target::Print
            }
            FenceKind::Collection => {
                if self.once(format!("c{}", fence_key(fence))) {
                    self.collections.push(fence.clone());
                }
                false
            }
            FenceKind::Image => {
                if let Some(src) = image_fence_src(fence) {
                    self.image(src);
                }
                false
            }
            _ => false,
        };
        if wants_figure && self.once(format!("f{}", fence_key(fence))) {
            self.figures.push(fence.clone());
        }
    }

    fn image(&mut self, url: String) {
        if self.once(format!("i{url}")) {
            self.images.push(url);
        }
    }

    fn inlines(&mut self, inlines: &[Inline]) {
        for inline in inlines {
            match inline {
                Inline::Emphasis { content }
                | Inline::Strong { content }
                | Inline::Strikethrough { content }
                | Inline::Highlight { content }
                | Inline::Subscript { content }
                | Inline::Superscript { content }
                | Inline::Link { content, .. } => self.inlines(content),
                Inline::Image { url, .. } => self.image(url.clone()),
                Inline::WikiLink { target, .. } => {
                    if self.once(format!("p{target}")) {
                        self.pages.push(target.clone());
                    }
                }
                Inline::Mention { handle } => {
                    if self.once(format!("m{handle}")) {
                        self.people.push(handle.clone());
                    }
                }
                Inline::IssueKey { key } => {
                    if self.once(format!("k{key}")) {
                        self.issues.push(key.clone());
                    }
                }
                Inline::Text { .. }
                | Inline::Code { .. }
                | Inline::SoftBreak
                | Inline::HardBreak
                | Inline::Date { .. } => {}
            }
        }
    }
}

fn image_fence_src(fence: &Fence) -> Option<String> {
    data::pairs(&fence.literal)
        .into_iter()
        .find(|(k, _)| k == "src" || k == "url")
        .map(|(_, v)| v)
}

struct FigureAnswer {
    asset: String,
    svg: FigureSvg,
    resolved: Option<Fence>,
}

struct Answers {
    figures: HashMap<String, Result<FigureAnswer, FigureError>>,
    collections: HashMap<String, Result<Snapshot, crate::ports::LiveError>>,
    issues: HashMap<String, IssueChip>,
    pages: HashMap<String, PageRef>,
    people: HashMap<String, String>,
}

struct Ctx<'a, 'p> {
    opts: &'a LowerOptions,
    answers: &'a Answers,
    session: &'a mut Session<'p>,
    anchor_seen: HashSet<String>,
}

impl Ctx<'_, '_> {
    fn warn(&mut self, location: impl Into<String>, message: impl Into<String>) {
        self.session
            .warnings
            .push(ExportWarning::new(location, message));
    }

    fn blocks(&mut self, blocks: &[Block]) -> Vec<Node> {
        let mut out = Vec::with_capacity(blocks.len());
        for block in blocks {
            self.block(block, &mut out);
        }
        out
    }

    fn block(&mut self, block: &Block, out: &mut Vec<Node>) {
        match block {
            Block::Paragraph { content, id } => {
                if let [Inline::Image { url, title, alt }] = content.as_slice() {
                    out.push(self.image_block(url, &inline_text(alt), title));
                    return;
                }
                let anchor = id.and_then(|id| self.block_anchor(&id.to_string()));
                out.push(Node::Paragraph {
                    spans: self.spans(content),
                    anchor,
                });
            }
            Block::Heading {
                level,
                content,
                attrs,
                id,
            } => {
                let text = inline_text(content);
                let anchor = id
                    .and_then(|id| self.block_anchor(&id.to_string()))
                    .unwrap_or_else(|| {
                        self.unique_anchor(attrs.id.clone().unwrap_or_else(|| slug(&text)))
                    });
                out.push(Node::Heading {
                    level: level.saturating_add(self.opts.heading_offset).clamp(1, 6),
                    number: None,
                    spans: self.spans(content),
                    anchor: Some(anchor),
                    outlined: true,
                });
            }
            Block::ThematicBreak => out.push(Node::Rule),
            Block::BlockQuote { blocks } => out.push(Node::Quote {
                blocks: self.blocks(blocks),
            }),
            Block::List(list) => out.push(Node::List {
                start: list.start,
                items: list.items.iter().map(|item| self.item(item)).collect(),
            }),
            Block::Fence(fence) => self.fence(fence, out),
            Block::Math { literal } => out.push(self.math(literal)),
            Block::Table(table) => out.push(self.table(table)),
            Block::Container(container) => self.container(container, out),
        }
    }

    fn item(&mut self, item: &ListItem) -> Item {
        Item {
            check: item.task,
            blocks: self.blocks(&item.blocks),
        }
    }

    fn block_anchor(&mut self, block_id: &str) -> Option<String> {
        self.opts
            .refs
            .blocks
            .get(block_id)
            .map(|(anchor, _)| anchor.clone())
            .filter(|anchor| self.session.anchors.insert(anchor.clone()))
    }

    fn unique_anchor(&mut self, base: String) -> String {
        let base = format!("{}{}", self.opts.anchor_prefix, base);
        let mut anchor = base.clone();
        let mut n = 1;
        while !self.session.anchors.insert(anchor.clone()) {
            n += 1;
            anchor = format!("{base}-{n}");
        }
        let _ = self.anchor_seen.insert(anchor.clone());
        anchor
    }

    fn math(&mut self, tex: &str) -> Node {
        match math::to_typst(tex) {
            Ok(typst) => Node::Math {
                tex: tex.trim().to_owned(),
                typst: Some(typst),
            },
            Err(error) => {
                self.warn("math", format!("{error}; the formula is shown as TeX"));
                Node::Math {
                    tex: tex.trim().to_owned(),
                    typst: None,
                }
            }
        }
    }

    fn image_block(&mut self, url: &str, alt: &str, caption: &str) -> Node {
        match self.session.by_url.get(url).cloned().flatten() {
            Some(asset) => Node::Image {
                asset,
                alt: alt.to_owned(),
                caption: caption.to_owned(),
            },
            None => Node::Paragraph {
                spans: vec![Span::Text {
                    text: format!("[Image: {}]", if alt.is_empty() { url } else { alt }),
                    style: Style {
                        italic: true,
                        ..Style::default()
                    },
                }],
                anchor: None,
            },
        }
    }

    // ---- fences -------------------------------------------------------

    fn fence(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let print = self.opts.target == Target::Print;
        match fence.kind() {
            FenceKind::Code => out.push(Node::Code {
                lang: fence.lang.clone(),
                text: fence.literal.trim_end_matches('\n').to_owned(),
            }),
            FenceKind::Chart => {
                if !self.figure(fence, out) {
                    self.chart_table(fence, out);
                }
            }
            FenceKind::Diagram | FenceKind::Architecture => {
                if !self.figure(fence, out) {
                    if fence.kind() == FenceKind::Architecture {
                        self.architecture_list(fence, out);
                    } else {
                        out.push(Node::Code {
                            lang: fence.lang.clone(),
                            text: fence.literal.trim_end_matches('\n').to_owned(),
                        });
                    }
                }
            }
            FenceKind::Timeline => {
                if !(print && self.figure(fence, out)) {
                    self.timeline_table(fence, out);
                }
            }
            FenceKind::Roadmap => {
                if !(print && self.figure(fence, out)) {
                    self.roadmap_list(fence, out);
                }
            }
            FenceKind::RiskMatrix => {
                if !(print && self.figure(fence, out)) {
                    self.risk_list(fence, out);
                }
            }
            FenceKind::Board => {
                if !(print && self.figure(fence, out)) {
                    self.board_blocks(fence, out);
                }
            }
            FenceKind::Table => out.push(self.csv_table(fence)),
            FenceKind::Calculator => out.push(self.calculator(fence)),
            FenceKind::Collection => self.collection(fence, out),
            FenceKind::Image => {
                let pairs = data::pairs(&fence.literal);
                let get = |key: &str| {
                    pairs
                        .iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                let src = image_fence_src(fence).unwrap_or_default();
                out.push(self.image_block(&src, &get("alt"), &get("caption")));
            }
        }
    }

    /// Pushes the drawn figure for `fence`; false when there is none.
    fn figure(&mut self, fence: &Fence, out: &mut Vec<Node>) -> bool {
        let key = fence_key(fence);
        match self.answers.figures.get(&key) {
            Some(Ok(answer)) => {
                let caption = if fence.kind() == FenceKind::Chart {
                    let source = answer.resolved.as_ref().unwrap_or(fence);
                    let title = data::chart(source).title;
                    if answer.resolved.is_some() {
                        format!("{title} (as of {})", self.opts.as_of).trim().to_owned()
                    } else {
                        title
                    }
                } else if fence.kind() == FenceKind::Roadmap {
                    format!("As of {}", self.opts.as_of)
                } else {
                    String::new()
                };
                // Pixels to points: 1 px = 0.75 pt.
                out.push(Node::Figure {
                    asset: answer.asset.clone(),
                    alt: answer.svg.alt.clone(),
                    caption,
                    width: answer.svg.width * 0.75,
                    height: answer.svg.height * 0.75,
                });
                true
            }
            Some(Err(FigureError::Unsupported)) | None => false,
            Some(Err(error)) => {
                let message = format!("{error}; shown as {}", fallback_name(fence.kind()));
                self.warn(fence.lang.clone(), message);
                false
            }
        }
    }

    fn chart_table(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let chart = data::chart(fence);
        if chart.series.is_empty() {
            out.push(Node::Code {
                lang: fence.lang.clone(),
                text: fence.literal.trim_end_matches('\n').to_owned(),
            });
            return;
        }
        if !chart.title.is_empty() {
            out.push(caption_paragraph(&chart.title));
        }
        let mut header = vec![Cell::text("")];
        header.extend(chart.series.iter().map(|(name, _)| Cell::text(name.clone())));
        let rows = chart
            .labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let mut row = vec![Cell::text(label.clone())];
                row.extend(chart.series.iter().map(|(_, values)| {
                    Cell::text(values.get(i).map_or_else(String::new, |v| {
                        format!("{}{}", data::display_number(*v), chart.unit)
                    }))
                }));
                row
            })
            .collect();
        let mut align = vec![Align::Left];
        align.extend(chart.series.iter().map(|_| Align::Right));
        out.push(Node::Table(Table {
            style: TableStyle::Data,
            align,
            header,
            rows,
        }));
    }

    fn architecture_list(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let arch = data::architecture(fence);
        let mut items: Vec<Item> = arch
            .lanes
            .iter()
            .map(|(lane, members)| text_item(&format!("{lane}: {}", members.join(", "))))
            .collect();
        items.extend(
            arch.links
                .iter()
                .map(|(from, to)| text_item(&format!("{from} → {to}"))),
        );
        out.push(Node::List { start: None, items });
    }

    fn timeline_table(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let events = data::timeline(fence);
        let has_detail = events.iter().any(|e| !e.detail.is_empty());
        let rows = events
            .into_iter()
            .map(|event| {
                let mut row = vec![Cell {
                    spans: vec![Span::bold(event.when)],
                    shade: None,
                }];
                if has_detail && !event.detail.is_empty() {
                    row.push(Cell {
                        spans: vec![
                            Span::plain(event.title),
                            Span::Break,
                            Span::Text {
                                text: event.detail,
                                style: Style {
                                    italic: true,
                                    ..Style::default()
                                },
                            },
                        ],
                        shade: None,
                    });
                } else {
                    row.push(Cell::text(event.title));
                }
                row
            })
            .collect();
        out.push(Node::Table(Table {
            style: TableStyle::Facts,
            align: vec![Align::Left, Align::Left],
            header: Vec::new(),
            rows,
        }));
    }

    fn roadmap_list(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let (_, bars) = data::roadmap(fence);
        out.push(caption_paragraph(&format!("Roadmap as of {}", self.opts.as_of)));
        let items = bars
            .into_iter()
            .map(|bar| {
                let when = match &bar.end {
                    Some(end) => format!("{} → {end}", bar.start),
                    None => format!("{} (milestone)", bar.start),
                };
                Item {
                    check: None,
                    blocks: vec![Node::Paragraph {
                        spans: vec![Span::bold(bar.name), Span::plain(format!(": {when}"))],
                        anchor: None,
                    }],
                }
            })
            .collect();
        out.push(Node::List { start: None, items });
    }

    fn risk_list(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let mut risks = data::risks(fence);
        risks.sort_by_key(|r| std::cmp::Reverse(u16::from(r.likelihood) * u16::from(r.impact)));
        let items = risks
            .into_iter()
            .map(|risk| Item {
                check: None,
                blocks: vec![Node::Paragraph {
                    spans: vec![
                        Span::bold(risk.title),
                        Span::plain(format!(
                            " — likelihood {}, impact {}",
                            risk.likelihood, risk.impact
                        )),
                    ],
                    anchor: None,
                }],
            })
            .collect();
        out.push(Node::List { start: None, items });
    }

    fn board_blocks(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        let columns = data::board(fence);
        let lowered: Vec<(String, Node)> = columns
            .into_iter()
            .map(|column| {
                let items = column
                    .cards
                    .into_iter()
                    .map(|(text, tag)| {
                        let mut spans = vec![Span::plain(text)];
                        if let Some(tag) = tag {
                            spans.push(Span::plain(" "));
                            spans.push(Span::Text {
                                text: format!("[{tag}]"),
                                style: Style {
                                    bold: true,
                                    ..Style::default()
                                },
                            });
                        }
                        Item {
                            check: None,
                            blocks: vec![Node::Paragraph {
                                spans,
                                anchor: None,
                            }],
                        }
                    })
                    .collect();
                (column.name, Node::List { start: None, items })
            })
            .collect();
        if self.opts.target == Target::Print {
            out.push(Node::Columns {
                columns: lowered
                    .into_iter()
                    .map(|(name, list)| vec![caption_paragraph(&name), list])
                    .collect(),
            });
        } else {
            out.extend(lowered.into_iter().map(|(name, list)| Node::Titled {
                title: vec![Span::plain(name)],
                blocks: vec![list],
            }));
        }
    }

    fn csv_table(&mut self, fence: &Fence) -> Node {
        let table = data::csv_table(fence);
        let numeric: Vec<bool> = (0..table.header.len())
            .map(|i| {
                !table.rows.is_empty()
                    && table.rows.iter().all(|row| {
                        row.get(i).is_some_and(|v| {
                            v.trim()
                                .trim_end_matches('%')
                                .replace(',', "")
                                .parse::<f64>()
                                .is_ok()
                        })
                    })
            })
            .collect();
        let mut rows: Vec<Vec<Cell>> = table
            .rows
            .iter()
            .map(|row| row.iter().map(|v| self.status_cell(v)).collect())
            .collect();
        if let Some(sum) = data::arg(fence, "sum") {
            if let Some(index) = table.header.iter().position(|h| *h == sum) {
                let total: f64 = table
                    .rows
                    .iter()
                    .filter_map(|row| row.get(index)?.replace(',', "").trim().parse::<f64>().ok())
                    .sum();
                let mut row: Vec<Cell> = table.header.iter().map(|_| Cell::text("")).collect();
                if let Some(first) = row.first_mut() {
                    *first = Cell {
                        spans: vec![Span::bold("Total")],
                        shade: None,
                    };
                }
                if let Some(cell) = row.get_mut(index) {
                    *cell = Cell {
                        spans: vec![Span::bold(data::display_number(total))],
                        shade: None,
                    };
                }
                rows.push(row);
            }
        }
        Node::Table(Table {
            style: TableStyle::Data,
            align: numeric
                .iter()
                .map(|&n| if n { Align::Right } else { Align::Left })
                .collect(),
            header: table.header.into_iter().map(Cell::text).collect(),
            rows,
        })
    }

    fn calculator(&mut self, fence: &Fence) -> Node {
        let (inputs, outputs) = data::calculator(fence);
        let mut rows: Vec<Vec<Cell>> = inputs
            .iter()
            .map(|input| {
                vec![
                    Cell::text(input.label.clone()),
                    Cell::text(data::display_number(input.value)),
                ]
            })
            .collect();
        for output in outputs {
            let value = match &output.value {
                Ok(value) if output.unit == "$" || output.unit == "€" || output.unit == "£" => {
                    format!("{}{}", output.unit, data::display_number(*value))
                }
                Ok(value) if output.unit.is_empty() => data::display_number(*value),
                Ok(value) => format!("{} {}", data::display_number(*value), output.unit),
                Err(error) => {
                    self.warn("calculator", error.clone());
                    format!("Error: {error}")
                }
            };
            rows.push(vec![
                Cell {
                    spans: vec![Span::bold(output.label)],
                    shade: None,
                },
                Cell {
                    spans: vec![Span::bold(value)],
                    shade: None,
                },
            ]);
        }
        Node::Table(Table {
            style: TableStyle::Facts,
            align: vec![Align::Left, Align::Right],
            header: Vec::new(),
            rows,
        })
    }

    fn collection(&mut self, fence: &Fence, out: &mut Vec<Node>) {
        match self.answers.collections.get(&fence_key(fence)) {
            Some(Ok(snapshot)) => {
                let title = if snapshot.title.is_empty() {
                    format!("As of {}", self.opts.as_of)
                } else {
                    format!("{} (as of {})", snapshot.title, self.opts.as_of)
                };
                out.push(caption_paragraph(&title));
                out.push(Node::Table(Table {
                    style: TableStyle::Data,
                    align: snapshot.columns.iter().map(|_| Align::Left).collect(),
                    header: snapshot.columns.iter().cloned().map(Cell::text).collect(),
                    rows: snapshot
                        .rows
                        .iter()
                        .map(|row| {
                            let mut cells: Vec<Cell> =
                                row.iter().map(|v| self.status_cell(v)).collect();
                            cells.resize_with(snapshot.columns.len(), || Cell::text(""));
                            cells
                        })
                        .collect(),
                }));
            }
            Some(Err(error)) => {
                self.warn("collection", error.to_string());
                out.push(unavailable_note(&self.opts.as_of));
            }
            None => out.push(unavailable_note(&self.opts.as_of)),
        }
    }

    // ---- tables -------------------------------------------------------

    fn table(&mut self, table: &vellum_format::Table) -> Node {
        let compare = is_compare_matrix(table);
        let cell = |ctx: &mut Self, inlines: &[Inline], first: bool| -> Cell {
            let text = inline_text(inlines);
            if compare && !first {
                if let Some(symbol) = compare_symbol(&text) {
                    return Cell::text(symbol);
                }
            }
            Cell {
                spans: ctx.spans(inlines),
                shade: status_tone(&text),
            }
        };
        let header = table
            .header
            .iter()
            .map(|c| Cell {
                spans: self.spans(c),
                shade: None,
            })
            .collect();
        let rows = table
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, c)| cell(self, c, i == 0))
                    .collect()
            })
            .collect();
        Node::Table(Table {
            style: TableStyle::Data,
            align: table
                .alignments
                .iter()
                .enumerate()
                .map(|(i, a)| match a {
                    Alignment::Center => Align::Center,
                    Alignment::Right => Align::Right,
                    Alignment::None if compare && i > 0 => Align::Center,
                    Alignment::Left | Alignment::None => Align::Left,
                })
                .collect(),
            header,
            rows,
        })
    }

    fn status_cell(&self, text: &str) -> Cell {
        Cell {
            spans: vec![Span::plain(text)],
            shade: status_tone(text),
        }
    }

    // ---- containers ---------------------------------------------------

    fn container(&mut self, container: &Container, out: &mut Vec<Node>) {
        let body: Vec<&Block> = container.sections.iter().flatten().collect();
        match container.kind() {
            ContainerKind::Callout => {
                let blocks = self.blocks_ref(&body);
                out.push(Node::Callout {
                    tone: Tone::from_callout(&container.name),
                    blocks,
                });
            }
            ContainerKind::Details => {
                let blocks = self.blocks_ref(&body);
                out.push(Node::Titled {
                    title: vec![Span::plain(container.arg.clone())],
                    blocks,
                });
            }
            ContainerKind::Columns => {
                let columns = container
                    .sections
                    .iter()
                    .map(|section| self.blocks(section))
                    .collect();
                out.push(Node::Columns { columns });
            }
            ContainerKind::Tabs => self.tabs(&body, out),
            ContainerKind::Toc => out.push(Node::Toc { depth: 3 }),
            ContainerKind::Stats => {
                let tiles = list_items(&body)
                    .map(|item| {
                        let text = item_text(item);
                        let mut parts = text.split(" · ").map(str::trim);
                        let value = parts.next().unwrap_or_default().to_owned();
                        let label = parts.next().unwrap_or_default().to_owned();
                        let delta = parts.collect::<Vec<_>>().join(" · ");
                        let trend = trend(&delta);
                        Stat {
                            value,
                            label,
                            delta,
                            trend,
                        }
                    })
                    .collect();
                out.push(Node::Stats { tiles });
            }
            ContainerKind::Facts => {
                let rows = list_items(&body)
                    .map(|item| {
                        let (key, value) = self.fact(item);
                        vec![
                            Cell {
                                spans: vec![Span::bold(key)],
                                shade: None,
                            },
                            value,
                        ]
                    })
                    .collect();
                out.push(Node::Table(Table {
                    style: TableStyle::Facts,
                    align: vec![Align::Left, Align::Left],
                    header: Vec::new(),
                    rows,
                }));
            }
            ContainerKind::Cards => {
                let cards = list_items(&body).map(|item| self.card(item)).collect();
                out.push(Node::Cards { cards });
            }
            ContainerKind::Progress => {
                let bars = list_items(&body)
                    .map(|item| {
                        let text = item_text(item);
                        let (label, pct) = text.rsplit_once('·').unwrap_or((&text, ""));
                        let percent = pct
                            .trim()
                            .trim_end_matches('%')
                            .trim()
                            .parse::<f64>()
                            .unwrap_or(0.0)
                            .clamp(0.0, 100.0);
                        Bar {
                            label: label.trim().to_owned(),
                            percent,
                        }
                    })
                    .collect();
                out.push(Node::Progress { bars });
            }
            ContainerKind::Link | ContainerKind::Embed => {
                let text = body
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph { content, .. } => Some(
                            content
                                .split(|i| matches!(i, Inline::SoftBreak | Inline::HardBreak))
                                .map(inline_text)
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let mut lines = text.iter().flat_map(|t| t.lines()).map(str::trim);
                let title = lines.next().unwrap_or(&container.arg).to_owned();
                let rest = lines.collect::<Vec<_>>().join(" ");
                out.push(Node::LinkCard {
                    url: container.arg.clone(),
                    title: if title.is_empty() {
                        container.arg.clone()
                    } else {
                        title
                    },
                    text: rest,
                });
            }
            ContainerKind::Steps | ContainerKind::Pages | ContainerKind::Unknown => {
                let blocks = self.blocks_ref(&body);
                out.extend(blocks);
            }
        }
    }

    fn blocks_ref(&mut self, blocks: &[&Block]) -> Vec<Node> {
        let mut out = Vec::new();
        for block in blocks {
            self.block(block, &mut out);
        }
        out
    }

    /// Tabs print one after another under their `##` titles.
    fn tabs(&mut self, body: &[&Block], out: &mut Vec<Node>) {
        let mut title: Option<Vec<Span>> = None;
        let mut current: Vec<&Block> = Vec::new();
        for block in body {
            if let Block::Heading { level: 2, content, .. } = block {
                if let Some(title) = title.take() {
                    let blocks = self.blocks_ref(&current);
                    out.push(Node::Titled { title, blocks });
                } else if !current.is_empty() {
                    out.extend(self.blocks_ref(&current));
                }
                current.clear();
                title = Some(self.spans(content));
            } else {
                current.push(block);
            }
        }
        match title {
            Some(title) => {
                let blocks = self.blocks_ref(&current);
                out.push(Node::Titled { title, blocks });
            }
            None => out.extend(self.blocks_ref(&current)),
        }
    }

    fn fact(&mut self, item: &ListItem) -> (String, Cell) {
        let inlines = item_inlines(item);
        // Split the inlines at the first ": " in a text run.
        let mut key = String::new();
        let mut rest: Vec<Inline> = Vec::new();
        let mut split = false;
        for inline in &inlines {
            if split {
                rest.push(inline.clone());
                continue;
            }
            match inline {
                Inline::Text { text } => match text.split_once(':') {
                    Some((k, v)) => {
                        key.push_str(k);
                        let v = v.trim_start();
                        if !v.is_empty() {
                            rest.push(Inline::Text { text: v.to_owned() });
                        }
                        split = true;
                    }
                    None => key.push_str(text),
                },
                other => key.push_str(&inline_text(std::slice::from_ref(other))),
            }
        }
        let text = inline_text(&rest);
        let cell = Cell {
            spans: self.spans(&rest),
            shade: status_tone(&text),
        };
        (key.trim().to_owned(), cell)
    }

    fn card(&mut self, item: &ListItem) -> Card {
        let inlines = item_inlines(item);
        let mut icon = String::new();
        let mut title = String::new();
        let mut rest: Vec<Inline> = Vec::new();
        for inline in &inlines {
            match inline {
                Inline::Strong { content } if title.is_empty() => title = inline_text(content),
                Inline::Text { text } if title.is_empty() => icon.push_str(text.trim()),
                other => rest.push(other.clone()),
            }
        }
        if let Some(Inline::Text { text }) = rest.first_mut() {
            *text = text.trim_start().to_owned();
        }
        if title.is_empty() {
            title = std::mem::take(&mut icon);
        }
        Card {
            icon,
            title,
            spans: self.spans(&rest),
        }
    }

    // ---- inlines ------------------------------------------------------

    fn spans(&mut self, inlines: &[Inline]) -> Vec<Span> {
        let mut out = Vec::new();
        self.inlines(inlines, Style::default(), &mut out);
        out
    }

    fn inlines(&mut self, inlines: &[Inline], style: Style, out: &mut Vec<Span>) {
        for inline in inlines {
            self.inline(inline, style, out);
        }
    }

    fn inline(&mut self, inline: &Inline, style: Style, out: &mut Vec<Span>) {
        match inline {
            Inline::Text { text } => push_text(out, text, style),
            Inline::Code { code } => push_text(
                out,
                code,
                Style {
                    code: true,
                    ..style
                },
            ),
            Inline::SoftBreak => push_text(out, " ", style),
            Inline::HardBreak => out.push(Span::Break),
            Inline::Emphasis { content } => self.inlines(
                content,
                Style {
                    italic: true,
                    ..style
                },
                out,
            ),
            Inline::Strong { content } => self.inlines(
                content,
                Style {
                    bold: true,
                    ..style
                },
                out,
            ),
            Inline::Strikethrough { content } => self.inlines(
                content,
                Style {
                    strike: true,
                    ..style
                },
                out,
            ),
            Inline::Highlight { content } => self.inlines(
                content,
                Style {
                    highlight: true,
                    ..style
                },
                out,
            ),
            Inline::Subscript { content } => self.inlines(
                content,
                Style { sub: true, ..style },
                out,
            ),
            Inline::Superscript { content } => self.inlines(
                content,
                Style { sup: true, ..style },
                out,
            ),
            Inline::Link { url, content, .. } => {
                let mut spans = Vec::new();
                self.inlines(content, style, &mut spans);
                self.link(url, spans, out);
            }
            Inline::Image { url, alt, .. } => match self.session.by_url.get(url).cloned().flatten() {
                Some(asset) => out.push(Span::Image {
                    asset,
                    alt: inline_text(alt),
                }),
                None => push_text(out, &inline_text(alt), style),
            },
            Inline::WikiLink { target, label } => {
                let page = self.answers.pages.get(target).cloned();
                let text = label
                    .clone()
                    .or_else(|| page.as_ref().map(|p| p.title.clone()))
                    .unwrap_or_else(|| target.clone());
                let url = page.map_or_else(
                    || {
                        if target.starts_with("vellum://") {
                            target.clone()
                        } else {
                            String::new()
                        }
                    },
                    |p| format!("vellum://doc/{}", p.id),
                );
                if url.is_empty() {
                    push_text(out, &text, style);
                } else {
                    self.link(&url, vec![Span::Text { text, style }], out);
                }
            }
            Inline::Mention { handle } => {
                let name = self
                    .answers
                    .people
                    .get(handle)
                    .cloned()
                    .unwrap_or_else(|| handle.clone());
                push_text(out, &format!("@{name}"), style);
            }
            Inline::IssueKey { key } => {
                let text = match self.answers.issues.get(key) {
                    Some(chip) => format!("{key} {} ({})", chip.title, chip.status),
                    None => key.clone(),
                };
                push_text(out, &text, style);
            }
            Inline::Date { date } => {
                let text = if date == "today" {
                    self.opts.as_of.clone()
                } else {
                    date.clone()
                };
                push_text(out, &text, style);
            }
        }
    }

    /// A link: a cross-reference when the target is in this export, a web
    /// link otherwise, or plain text when it has nowhere to go.
    fn link(&mut self, url: &str, spans: Vec<Span>, out: &mut Vec<Span>) {
        if let Some(rest) = url.strip_prefix("vellum://doc/") {
            let (doc, fragment) = rest.split_once('#').unwrap_or((rest, ""));
            let target = if fragment.is_empty() {
                self.opts.refs.docs.get(doc)
            } else {
                self.opts
                    .refs
                    .blocks
                    .get(fragment)
                    .or_else(|| self.opts.refs.docs.get(doc))
            };
            if let Some((anchor, text)) = target {
                out.push(Span::Xref {
                    anchor: anchor.clone(),
                    text: text.clone(),
                });
                return;
            }
            match &self.opts.base_url {
                Some(base) => {
                    let mut href = format!("{}/d/{doc}", base.trim_end_matches('/'));
                    if !fragment.is_empty() {
                        let _ = write!(href, "#{fragment}");
                    }
                    out.push(Span::Link { url: href, spans });
                }
                None => out.extend(spans),
            }
            return;
        }
        if let Some(fragment) = url.strip_prefix('#') {
            if let Some((anchor, text)) = self.opts.refs.blocks.get(fragment) {
                out.push(Span::Xref {
                    anchor: anchor.clone(),
                    text: text.clone(),
                });
                return;
            }
        }
        if is_safe_url(url) {
            out.push(Span::Link {
                url: url.to_owned(),
                spans,
            });
        } else {
            out.extend(spans);
        }
    }
}

/// Only web and mail links survive into exports; `javascript:`, `file:`
/// and other schemes become plain text.
fn is_safe_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")
}

fn push_text(out: &mut Vec<Span>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(Span::Text {
        text: last,
        style: last_style,
    }) = out.last_mut()
    {
        if *last_style == style {
            last.push_str(text);
            return;
        }
    }
    out.push(Span::Text {
        text: text.to_owned(),
        style,
    });
}

fn text_item(text: &str) -> Item {
    Item {
        check: None,
        blocks: vec![Node::Paragraph {
            spans: vec![Span::plain(text)],
            anchor: None,
        }],
    }
}

fn caption_paragraph(text: &str) -> Node {
    Node::Paragraph {
        spans: vec![Span::bold(text)],
        anchor: None,
    }
}

fn unavailable_note(as_of: &str) -> Node {
    Node::Callout {
        tone: Tone::Note,
        blocks: vec![Node::Paragraph {
            spans: vec![Span::plain(format!(
                "Live data was not available when this was exported on {as_of}."
            ))],
            anchor: None,
        }],
    }
}

const fn fallback_name(kind: FenceKind) -> &'static str {
    match kind {
        FenceKind::Chart => "a data table",
        FenceKind::Timeline => "a dated table",
        FenceKind::Roadmap => "a dated list",
        FenceKind::RiskMatrix => "a list of risks",
        FenceKind::Board => "columns of cards",
        FenceKind::Architecture => "a list",
        _ => "its source",
    }
}

fn list_items<'a>(body: &'a [&'a Block]) -> impl Iterator<Item = &'a ListItem> {
    body.iter()
        .filter_map(|block| match block {
            Block::List(list) => Some(list.items.iter()),
            _ => None,
        })
        .flatten()
}

fn item_inlines(item: &ListItem) -> Vec<Inline> {
    item.blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { content, .. } => Some(content.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn item_text(item: &ListItem) -> String {
    inline_text(&item_inlines(item))
}

fn trend(delta: &str) -> Option<Tone> {
    if delta.contains('▲') {
        Some(Tone::Good)
    } else if delta.contains('▼') {
        Some(Tone::Crit)
    } else if delta.contains('▬') {
        Some(Tone::Note)
    } else {
        None
    }
}

/// The chip shade for a cell that holds only a status word.
fn status_tone(text: &str) -> Option<Tone> {
    match text.trim().to_lowercase().as_str() {
        "done" | "complete" | "completed" | "verified" | "shipped" | "on track" | "approved"
        | "yes" | "pass" | "passed" => Some(Tone::Good),
        "in review" | "in progress" | "started" | "at risk" | "review" | "pending" | "doing" => {
            Some(Tone::Warn)
        }
        "blocked" | "off track" | "failed" | "fail" | "canceled" | "cancelled" | "outdated" => {
            Some(Tone::Crit)
        }
        "todo" | "to do" | "backlog" | "draft" | "planned" | "not started" => Some(Tone::Note),
        _ => None,
    }
}

fn compare_symbol(text: &str) -> Option<&'static str> {
    match text.trim().to_lowercase().as_str() {
        "✓" | "yes" | "y" => Some("✓"),
        "✗" | "no" | "n" => Some("✗"),
        "◐" | "part" | "partial" | "partly" => Some("◐"),
        _ => None,
    }
}

/// A compare matrix is a GFM table whose cells after the first column are
/// all ✓ ✗ ◐ or yes / no / part.
fn is_compare_matrix(table: &vellum_format::Table) -> bool {
    !table.rows.is_empty()
        && table.alignments.len() > 1
        && table.rows.iter().all(|row| {
            row.iter()
                .skip(1)
                .all(|cell| compare_symbol(&inline_text(cell)).is_some())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_url_safe() {
        assert_eq!(slug("Rollback & recovery (v2)"), "rollback-recovery-v2");
        assert_eq!(slug("   "), "section");
        assert_eq!(slug("Überblick"), "überblick");
    }

    #[test]
    fn status_words_get_chip_tones() {
        assert_eq!(status_tone(" Done "), Some(Tone::Good));
        assert_eq!(status_tone("In review"), Some(Tone::Warn));
        assert_eq!(status_tone("Blocked"), Some(Tone::Crit));
        assert_eq!(status_tone("Priya"), None);
    }

    #[test]
    fn unsafe_link_schemes_are_dropped() {
        assert!(is_safe_url("https://northstar.dev"));
        assert!(is_safe_url("mailto:priya@northstar.dev"));
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("file:///etc/passwd"));
    }

    #[test]
    fn raster_sizes_come_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(Media::Png, &png), (Some(640), Some(480)));
        let gif = b"GIF89a\x40\x01\xf0\x00".to_vec();
        assert_eq!(image_size(Media::Gif, &gif), (Some(320), Some(240)));
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00,
            0x20, 0x00, 0x40,
        ];
        assert_eq!(image_size(Media::Jpeg, &jpeg), (Some(64), Some(32)));
    }
}
