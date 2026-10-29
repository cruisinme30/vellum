// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Word export: a [`Flow`] written as an Office Open XML package.
//!
//! The writer emits `WordprocessingML` directly (decision D71 in ADR 0014)
//! so that it can use what Word offers and the PRD asks for: heading
//! styles with outline levels, real numbered and bulleted lists, real
//! tables with shaded status cells, SVG figures with a PNG fallback,
//! OMML equations, bookmarks with `PAGEREF` cross-references, and a
//! contents field that Word refreshes on open. Output is deterministic:
//! the same flow gives the same bytes.

mod parts;
mod raster;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::time::Instant;

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

pub use raster::rasterize_svg;

use crate::error::ExportError;
use crate::flow::{
    Align, Asset, Cell, Flow, Item, Layout, Media, Node, Span, Stat, Style, Table, TableStyle,
    Tone, plain_text,
};
use crate::math;
use crate::metrics;

/// Page sizes for Word output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PageSize {
    /// ISO A4.
    #[default]
    A4,
    /// US Letter.
    Letter,
}

/// How equations are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MathMode {
    /// Native Word equations (OMML).
    #[default]
    Equation,
    /// The `TeX` source in a code paragraph, for targets that do not read
    /// equations.
    Source,
}

/// Settings for one Word export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocxOptions {
    /// Page size.
    pub page: PageSize,
    /// Embed SVG figures (with a PNG fallback). Off for Google Docs, which
    /// reads only the PNG.
    pub svg: bool,
    /// How equations are written.
    pub math: MathMode,
    /// Body font family.
    pub body_font: String,
    /// Code font family.
    pub mono_font: String,
    /// Link colour, `#RRGGBB`.
    pub link_color: String,
}

impl Default for DocxOptions {
    fn default() -> Self {
        Self {
            page: PageSize::A4,
            svg: true,
            math: MathMode::Equation,
            body_font: "Roboto".to_owned(),
            mono_font: "Roboto Mono".to_owned(),
            link_color: "#4F5B92".to_owned(),
        }
    }
}

/// Escapes text for XML content and attribute values.
#[must_use]
pub fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 cannot carry are dropped.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// English Metric Units per point.
const EMU_PER_PT: f64 = 12_700.0;
/// EMU per CSS pixel (96 dpi).
const EMU_PER_PX: f64 = 9_525.0;

/// Writes `flow` as a `.docx` file.
///
/// # Errors
///
/// [`ExportError::Write`] when the package cannot be written.
#[tracing::instrument(name = "export.docx", skip_all, fields(blocks = flow.blocks.len(), assets = flow.assets.len()))]
pub fn write(flow: &Flow, options: &DocxOptions) -> Result<Vec<u8>, ExportError> {
    let started = Instant::now();
    let result = write_inner(flow, options);
    metrics::export_done("docx", started, result.as_ref().map(Vec::len));
    result
}

fn write_inner(flow: &Flow, options: &DocxOptions) -> Result<Vec<u8>, ExportError> {
    let mut doc = Writer::new(flow, options);
    doc.document();
    doc.package()
}

/// Content width in twentieths of a point, for 2.54 cm margins.
const fn content_width_twips(page: PageSize) -> u32 {
    match page {
        PageSize::A4 => 11_906 - 2 * 1_440,
        PageSize::Letter => 12_240 - 2 * 1_440,
    }
}

struct Media0 {
    path: String,
    bytes: Vec<u8>,
    media: Media,
}

struct Writer<'a> {
    flow: &'a Flow,
    options: &'a DocxOptions,
    body: String,
    rels: Vec<(String, String, String, bool)>,
    media: Vec<Media0>,
    image_rels: HashMap<String, (String, Option<String>)>,
    link_rels: HashMap<String, String>,
    bookmarks: HashMap<String, String>,
    nums: Vec<(u32, u32, u64)>,
    next_bookmark: u32,
    next_drawing: u32,
}

impl<'a> Writer<'a> {
    fn new(flow: &'a Flow, options: &'a DocxOptions) -> Self {
        let mut writer = Self {
            flow,
            options,
            body: String::new(),
            rels: Vec::new(),
            media: Vec::new(),
            image_rels: HashMap::new(),
            link_rels: HashMap::new(),
            bookmarks: HashMap::new(),
            nums: Vec::new(),
            next_bookmark: 1,
            next_drawing: 1,
        };
        let _ = writer.rel("styles", "styles.xml");
        let _ = writer.rel("numbering", "numbering.xml");
        let _ = writer.rel("settings", "settings.xml");
        let _ = writer.rel("footer", "footer1.xml");
        writer.collect_anchors(&flow.blocks);
        writer
    }

    /// Adds an internal relationship and returns its ID.
    fn rel(&mut self, kind: &str, target: &str) -> String {
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push((
            id.clone(),
            format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"),
            target.to_owned(),
            false,
        ));
        id
    }

    fn hyperlink_rel(&mut self, url: &str) -> String {
        if let Some(id) = self.link_rels.get(url) {
            return id.clone();
        }
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push((
            id.clone(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
                .to_owned(),
            url.to_owned(),
            true,
        ));
        let _ = self.link_rels.insert(url.to_owned(), id.clone());
        id
    }

    /// Word bookmark names must start with a letter or underscore and be at
    /// most 40 characters; anchors are mapped to `_V<n>` (hidden bookmarks).
    fn collect_anchors(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Heading {
                    anchor: Some(anchor),
                    ..
                }
                | Node::Paragraph {
                    anchor: Some(anchor),
                    ..
                } => {
                    let n = self.bookmarks.len() + 1;
                    let _ = self
                        .bookmarks
                        .entry(anchor.clone())
                        .or_insert_with(|| format!("_V{n}"));
                }
                Node::Quote { blocks }
                | Node::Callout { blocks, .. }
                | Node::Titled { blocks, .. } => self.collect_anchors(blocks),
                Node::List { items, .. } => {
                    for item in items {
                        self.collect_anchors(&item.blocks);
                    }
                }
                Node::Columns { columns } => {
                    for column in columns {
                        self.collect_anchors(column);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- package ------------------------------------------------------

    fn package(mut self) -> Result<Vec<u8>, ExportError> {
        let (width, height) = match self.options.page {
            PageSize::A4 => (11_906, 16_838),
            PageSize::Letter => (12_240, 15_840),
        };
        let footer = self
            .rels
            .iter()
            .find(|r| r.2 == "footer1.xml")
            .map(|r| r.0.clone())
            .unwrap_or_default();
        let _ = write!(
            self.body,
            concat!(
                r#"<w:sectPr><w:footerReference w:type="default" r:id="{footer}"/>"#,
                r#"<w:pgSz w:w="{w}" w:h="{h}"/>"#,
                r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>"#,
                "</w:sectPr>"
            ),
            footer = footer,
            w = width,
            h = height
        );
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {}><w:body>{}</w:body></w:document>"#,
            parts::DOCUMENT_NAMESPACES,
            self.body
        );
        let mut rels = String::from(concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#
        ));
        for (id, kind, target, external) in &self.rels {
            let _ = write!(
                rels,
                r#"<Relationship Id="{id}" Type="{kind}" Target="{}"{}/>"#,
                xml(target),
                if *external {
                    r#" TargetMode="External""#
                } else {
                    ""
                }
            );
        }
        rels.push_str("</Relationships>");

        let media_types: Vec<Media> = self.media.iter().map(|m| m.media).collect();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let put = |zip: &mut ZipWriter<Cursor<Vec<u8>>>,
                       name: &str,
                       bytes: &[u8],
                       options: SimpleFileOptions|
         -> Result<(), ExportError> {
            zip.start_file(name, options)
                .map_err(|e| ExportError::Write(e.to_string()))?;
            zip.write_all(bytes)
                .map_err(|e| ExportError::Write(e.to_string()))
        };
        put(
            &mut zip,
            "[Content_Types].xml",
            parts::content_types(&media_types).as_bytes(),
            deflate,
        )?;
        put(&mut zip, "_rels/.rels", parts::PACKAGE_RELS.as_bytes(), deflate)?;
        put(
            &mut zip,
            "docProps/core.xml",
            parts::core(&self.flow.title, &self.flow.as_of).as_bytes(),
            deflate,
        )?;
        put(&mut zip, "docProps/app.xml", parts::APP.as_bytes(), deflate)?;
        put(&mut zip, "word/document.xml", document.as_bytes(), deflate)?;
        put(
            &mut zip,
            "word/_rels/document.xml.rels",
            rels.as_bytes(),
            deflate,
        )?;
        put(
            &mut zip,
            "word/styles.xml",
            parts::styles(
                &self.options.body_font,
                &self.options.mono_font,
                &self.options.link_color,
            )
            .as_bytes(),
            deflate,
        )?;
        put(
            &mut zip,
            "word/numbering.xml",
            parts::numbering(&self.nums).as_bytes(),
            deflate,
        )?;
        put(&mut zip, "word/settings.xml", parts::SETTINGS.as_bytes(), deflate)?;
        put(&mut zip, "word/footer1.xml", parts::FOOTER.as_bytes(), deflate)?;
        for media in &self.media {
            let options = if media.media == Media::Svg {
                deflate
            } else {
                stored
            };
            put(&mut zip, &format!("word/{}", media.path), &media.bytes, options)?;
        }
        let cursor = zip
            .finish()
            .map_err(|e| ExportError::Write(e.to_string()))?;
        Ok(cursor.into_inner())
    }

    // ---- blocks -------------------------------------------------------

    fn document(&mut self) {
        let flow = self.flow;
        match flow.layout {
            Layout::Book => {
                self.styled_paragraph("Title", &[Span::plain(flow.title.clone())]);
                self.styled_paragraph(
                    "Subheading",
                    &[Span::plain(format!("As of {}", flow.as_of))],
                );
                self.page_break();
            }
            Layout::Document if flow.show_title && !flow.title.is_empty() => {
                self.styled_paragraph("Title", &[Span::plain(flow.title.clone())]);
            }
            Layout::Document => {}
        }
        self.blocks(&flow.blocks, 0);
    }

    fn blocks(&mut self, nodes: &[Node], depth: u32) {
        for node in nodes {
            self.node(node, depth);
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per node kind
    fn node(&mut self, node: &Node, depth: u32) {
        match node {
            Node::Heading {
                level,
                number,
                spans,
                anchor,
                ..
            } => {
                let mut runs = String::new();
                if let Some(number) = number {
                    self.runs(&[Span::plain(format!("{number}\u{a0}\u{a0}"))], &mut runs);
                }
                self.runs(spans, &mut runs);
                let runs = self.bookmarked(anchor.as_deref(), runs);
                let _ = write!(
                    self.body,
                    r#"<w:p><w:pPr><w:pStyle w:val="Heading{level}"/></w:pPr>{runs}</w:p>"#
                );
            }
            Node::Paragraph { spans, anchor } => {
                let mut runs = String::new();
                self.runs(spans, &mut runs);
                let runs = self.bookmarked(anchor.as_deref(), runs);
                let _ = write!(self.body, "<w:p>{runs}</w:p>");
            }
            Node::Quote { blocks } => {
                for block in blocks {
                    match block {
                        Node::Paragraph { spans, .. } => self.styled_paragraph("Quote", spans),
                        other => self.node(other, depth),
                    }
                }
            }
            Node::List { start, items } => self.list(*start, items, depth),
            Node::Code { text, .. } => self.code(text),
            Node::Math { tex, .. } => self.math(tex),
            Node::Table(table) => self.table(table),
            Node::Rule => self.body.push_str(concat!(
                r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="C5C6D0"/></w:pBdr></w:pPr></w:p>"#
            )),
            Node::Callout { tone, blocks } => self.callout(*tone, blocks, depth),
            Node::Titled { title, blocks } => {
                self.styled_paragraph("Subheading", title);
                self.blocks(blocks, depth);
            }
            Node::Columns { columns } => {
                let width = content_width_twips(self.options.page);
                let n = u32::try_from(columns.len().max(1)).unwrap_or(1);
                self.table_open("VellumPlain", &vec![width / n; n as usize], false);
                self.body.push_str("<w:tr>");
                for column in columns {
                    self.cell_open(None, None);
                    let before = self.body.len();
                    self.blocks(column, depth);
                    self.ensure_paragraph(before);
                    self.body.push_str("</w:tc>");
                }
                self.body.push_str("</w:tr></w:tbl>");
                self.spacer();
            }
            Node::Stats { tiles } => self.stats(tiles),
            Node::Cards { cards } => {
                let per_row = cards.len().clamp(1, 3);
                let width = content_width_twips(self.options.page);
                let n = u32::try_from(per_row).unwrap_or(1);
                self.table_open("VellumTable", &vec![width / n; per_row], false);
                for row in cards.chunks(per_row) {
                    self.body.push_str("<w:tr>");
                    for card in row {
                        self.cell_open(None, None);
                        let mut title = String::new();
                        let heading = if card.icon.is_empty() {
                            card.title.clone()
                        } else {
                            format!("{} {}", card.icon, card.title)
                        };
                        self.runs(&[Span::bold(heading)], &mut title);
                        let _ = write!(self.body, "<w:p>{title}</w:p>");
                        let mut text = String::new();
                        self.runs(&card.spans, &mut text);
                        let _ = write!(self.body, "<w:p>{text}</w:p></w:tc>");
                    }
                    for _ in row.len()..per_row {
                        self.cell_open(None, None);
                        self.body.push_str("<w:p/></w:tc>");
                    }
                    self.body.push_str("</w:tr>");
                }
                self.body.push_str("</w:tbl>");
                self.spacer();
            }
            Node::Progress { bars } => {
                for bar in bars {
                    let text = format!("{} — {}%", bar.label, crate::data::display_number(bar.percent));
                    self.styled_paragraph("Normal", &[Span::plain(text)]);
                }
            }
            Node::LinkCard { url, title, text } => {
                let mut runs = String::new();
                self.link_runs(url, &[Span::bold(title.clone())], &mut runs);
                if !text.is_empty() {
                    runs.push_str("<w:r><w:br/></w:r>");
                    self.runs(&[Span::plain(text.clone())], &mut runs);
                }
                runs.push_str("<w:r><w:br/></w:r>");
                self.runs(
                    &[Span::Text {
                        text: url.clone(),
                        style: Style {
                            italic: true,
                            ..Style::default()
                        },
                    }],
                    &mut runs,
                );
                let _ = write!(self.body, "<w:p>{runs}</w:p>");
            }
            Node::Figure {
                asset,
                alt,
                caption,
                width,
                height,
            } => {
                self.picture_paragraph(asset, alt, Some((*width, *height)));
                if !caption.is_empty() {
                    self.styled_paragraph("Caption", &[Span::plain(caption.clone())]);
                }
            }
            Node::Image {
                asset,
                alt,
                caption,
            } => {
                self.picture_paragraph(asset, alt, None);
                if !caption.is_empty() {
                    self.styled_paragraph("Caption", &[Span::plain(caption.clone())]);
                }
            }
            Node::Toc { depth: toc_depth } => self.toc(*toc_depth),
            Node::PageBreak => self.page_break(),
        }
    }

    fn bookmarked(&mut self, anchor: Option<&str>, runs: String) -> String {
        match anchor.and_then(|a| self.bookmarks.get(a)).cloned() {
            Some(name) => {
                let id = self.next_bookmark;
                self.next_bookmark += 1;
                format!(
                    r#"<w:bookmarkStart w:id="{id}" w:name="{name}"/>{runs}<w:bookmarkEnd w:id="{id}"/>"#
                )
            }
            None => runs,
        }
    }

    fn styled_paragraph(&mut self, style: &str, spans: &[Span]) {
        let mut runs = String::new();
        self.runs(spans, &mut runs);
        let _ = write!(
            self.body,
            r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr>{runs}</w:p>"#
        );
    }

    fn page_break(&mut self) {
        self.body
            .push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
    }

    /// Word needs a paragraph between two tables and at the end of a cell.
    fn spacer(&mut self) {
        self.body
            .push_str(r#"<w:p><w:pPr><w:spacing w:after="0"/></w:pPr></w:p>"#);
    }

    fn ensure_paragraph(&mut self, before: usize) {
        let added = self.body.get(before..).unwrap_or_default();
        if !added.contains("<w:p>") && !added.contains("<w:p ") && !added.contains("<w:p/>") {
            self.body.push_str("<w:p/>");
        } else if added.ends_with("</w:tbl>") {
            self.body.push_str("<w:p/>");
        }
    }

    fn list(&mut self, start: Option<u64>, items: &[Item], depth: u32) {
        let tasks = items.iter().any(|i| i.check.is_some());
        let abstract_id = if tasks {
            3
        } else if start.is_some() {
            2
        } else {
            1
        };
        let num = u32::try_from(self.nums.len() + 1).unwrap_or(u32::MAX);
        self.nums.push((num, abstract_id, start.unwrap_or(1)));
        let level = depth.min(8);
        for item in items {
            let mut first = true;
            for block in &item.blocks {
                match block {
                    Node::Paragraph { spans, anchor } if first => {
                        let mut runs = String::new();
                        if let Some(done) = item.check {
                            self.runs(
                                &[Span::plain(if done { "☑ " } else { "☐ " })],
                                &mut runs,
                            );
                        }
                        self.runs(spans, &mut runs);
                        let runs = self.bookmarked(anchor.as_deref(), runs);
                        let _ = write!(
                            self.body,
                            concat!(
                                r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/>"#,
                                r#"<w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr>{runs}</w:p>"#
                            ),
                            level = level,
                            num = num,
                            runs = runs
                        );
                    }
                    Node::Paragraph { spans, .. } => {
                        let mut runs = String::new();
                        self.runs(spans, &mut runs);
                        let _ = write!(
                            self.body,
                            r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:ind w:left="{}"/></w:pPr>{runs}</w:p>"#,
                            720 * (level + 1)
                        );
                    }
                    Node::List { start, items } => self.list(*start, items, depth + 1),
                    other => self.node(other, depth + 1),
                }
                first = false;
            }
            if item.blocks.is_empty() {
                let _ = write!(
                    self.body,
                    r#"<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr></w:p>"#
                );
            }
        }
    }

    fn code(&mut self, text: &str) {
        let mut runs = String::new();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                runs.push_str("<w:r><w:br/></w:r>");
            }
            if !line.is_empty() {
                let _ = write!(
                    runs,
                    r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
                    xml(line)
                );
            }
        }
        let _ = write!(
            self.body,
            r#"<w:p><w:pPr><w:pStyle w:val="Code"/></w:pPr>{runs}</w:p>"#
        );
    }

    fn math(&mut self, tex: &str) {
        if self.options.math == MathMode::Equation {
            if let Ok(omml) = math::to_omml(tex) {
                let _ = write!(
                    self.body,
                    "<w:p><m:oMathPara>{omml}</m:oMathPara></w:p>"
                );
                return;
            }
        }
        self.code(tex);
    }

    fn table_open(&mut self, style: &str, widths: &[u32], fixed: bool) {
        let _ = write!(
            self.body,
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="{style}"/><w:tblW w:w="5000" w:type="pct"/>{}<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="0" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid>"#,
            if fixed {
                r#"<w:tblLayout w:type="fixed"/>"#
            } else {
                ""
            }
        );
        for width in widths {
            let _ = write!(self.body, r#"<w:gridCol w:w="{width}"/>"#);
        }
        self.body.push_str("</w:tblGrid>");
    }

    fn cell_open(&mut self, fill: Option<&str>, left_border: Option<&str>) {
        self.body.push_str("<w:tc><w:tcPr>");
        if let Some(color) = left_border {
            let _ = write!(
                self.body,
                r#"<w:tcBorders><w:left w:val="single" w:sz="24" w:space="0" w:color="{color}"/></w:tcBorders>"#
            );
        }
        if let Some(fill) = fill {
            let _ = write!(
                self.body,
                r#"<w:shd w:val="clear" w:color="auto" w:fill="{fill}"/>"#
            );
        }
        self.body.push_str("</w:tcPr>");
    }

    fn table(&mut self, table: &Table) {
        let columns = table.columns().max(1);
        let width = content_width_twips(self.options.page);
        let n = u32::try_from(columns).unwrap_or(1);
        let widths: Vec<u32> = if table.style == TableStyle::Facts && columns == 2 {
            vec![width * 35 / 100, width * 65 / 100]
        } else {
            vec![width / n; columns]
        };
        self.table_open("VellumTable", &widths, false);
        if !table.header.is_empty() {
            self.body
                .push_str("<w:tr><w:trPr><w:tblHeader/></w:trPr>");
            for (i, cell) in table.header.iter().enumerate() {
                self.cell_open(Some("F0F2F9"), None);
                let mut runs = String::new();
                let bold: Vec<Span> = cell.spans.iter().map(embolden).collect();
                self.runs(&bold, &mut runs);
                let _ = write!(
                    self.body,
                    "<w:p>{}{runs}</w:p></w:tc>",
                    align_ppr(table.align.get(i).copied())
                );
            }
            self.body.push_str("</w:tr>");
        }
        for row in &table.rows {
            self.body.push_str("<w:tr>");
            for (i, cell) in row.iter().enumerate() {
                self.table_cell(cell, table.align.get(i).copied());
            }
            self.body.push_str("</w:tr>");
        }
        self.body.push_str("</w:tbl>");
        self.spacer();
    }

    fn table_cell(&mut self, cell: &Cell, align: Option<Align>) {
        let fill = cell.shade.map(tone_bg);
        self.cell_open(fill, None);
        let mut runs = String::new();
        self.runs(&cell.spans, &mut runs);
        let _ = write!(self.body, "<w:p>{}{runs}</w:p></w:tc>", align_ppr(align));
    }

    fn callout(&mut self, tone: Tone, blocks: &[Node], depth: u32) {
        let width = content_width_twips(self.options.page);
        self.table_open("VellumPlain", &[width], false);
        self.body.push_str("<w:tr>");
        self.cell_open(Some(tone_bg(tone)), Some(tone_fg(tone)));
        let label = format!(
            r#"<w:p><w:r><w:rPr><w:b/><w:color w:val="{}"/></w:rPr><w:t>{}</w:t></w:r></w:p>"#,
            tone_fg(tone),
            tone.label()
        );
        self.body.push_str(&label);
        let before = self.body.len();
        self.blocks(blocks, depth);
        self.ensure_paragraph(before);
        self.body.push_str("</w:tc></w:tr></w:tbl>");
        self.spacer();
    }

    fn stats(&mut self, tiles: &[Stat]) {
        let width = content_width_twips(self.options.page);
        let per_row = tiles.len().clamp(1, 4);
        let n = u32::try_from(per_row).unwrap_or(1);
        self.table_open("VellumTable", &vec![width / n; per_row], false);
        for row in tiles.chunks(per_row) {
            self.body.push_str("<w:tr>");
            for tile in row {
                self.cell_open(Some("F0F2F9"), None);
                let _ = write!(
                    self.body,
                    r#"<w:p><w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    xml(&tile.value)
                );
                let _ = write!(
                    self.body,
                    r#"<w:p><w:r><w:rPr><w:color w:val="46464F"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                    xml(&tile.label)
                );
                if !tile.delta.is_empty() {
                    let color = tile.trend.map_or("46464F", tone_fg);
                    let _ = write!(
                        self.body,
                        r#"<w:p><w:r><w:rPr><w:color w:val="{color}"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                        xml(&tile.delta)
                    );
                }
                self.body.push_str("</w:tc>");
            }
            for _ in row.len()..per_row {
                self.cell_open(None, None);
                self.body.push_str("<w:p/></w:tc>");
            }
            self.body.push_str("</w:tr>");
        }
        self.body.push_str("</w:tbl>");
        self.spacer();
    }

    /// A contents field Word refreshes on open, with entries already filled
    /// in for viewers that do not update fields.
    fn toc(&mut self, depth: u8) {
        self.body.push_str(concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="TOCHeading"/></w:pPr><w:r><w:t>Contents</w:t></w:r></w:p>"#
        ));
        let mut entries = Vec::new();
        collect_headings(&self.flow.blocks, depth, &mut entries);
        let _ = write!(
            self.body,
            concat!(
                r#"<w:p><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r>"#,
                r#"<w:r><w:instrText xml:space="preserve"> TOC \o "1-{depth}" \h \z \u </w:instrText></w:r>"#,
                r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r></w:p>"#
            ),
            depth = depth.clamp(1, 6)
        );
        for (level, text, anchor) in entries {
            let style = format!("TOC{}", level.min(3));
            let link = anchor
                .and_then(|a| self.bookmarks.get(&a))
                .cloned()
                .unwrap_or_default();
            let _ = write!(
                self.body,
                r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:hyperlink w:anchor="{}" w:history="1"><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:hyperlink></w:p>"#,
                xml(&link),
                xml(&text)
            );
        }
        self.body.push_str(
            r#"<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
    }

    // ---- images -------------------------------------------------------

    fn asset(&self, name: &str) -> Option<&'a Asset> {
        self.flow.asset(name)
    }

    /// Relationship IDs (raster, optional SVG) for an asset, adding the
    /// media files on first use.
    fn image_rel(&mut self, asset: &Asset) -> Option<(String, Option<String>)> {
        if let Some(rels) = self.image_rels.get(&asset.name) {
            return Some(rels.clone());
        }
        let index = self.media.len() + 1;
        let rels = if asset.media == Media::Svg {
            let png = rasterize_svg(&asset.bytes, 2.0)?;
            let png_path = format!("media/image{index}.png");
            let png_rel = self.rel("image", &png_path);
            self.media.push(Media0 {
                path: png_path,
                bytes: png,
                media: Media::Png,
            });
            let svg_rel = if self.options.svg {
                let svg_path = format!("media/image{index}.svg");
                let rel = self.rel("image", &svg_path);
                self.media.push(Media0 {
                    path: svg_path,
                    bytes: asset.bytes.clone(),
                    media: Media::Svg,
                });
                Some(rel)
            } else {
                None
            };
            (png_rel, svg_rel)
        } else {
            let path = format!("media/image{index}.{}", asset.media.extension());
            let rel = self.rel("image", &path);
            self.media.push(Media0 {
                path,
                bytes: asset.bytes.clone(),
                media: asset.media,
            });
            (rel, None)
        };
        let _ = self.image_rels.insert(asset.name.clone(), rels.clone());
        Some(rels)
    }

    /// The drawing size in EMU: the figure size in points, or the image's
    /// pixel size, scaled down to the content width.
    fn extent(&self, asset: &Asset, points: Option<(f64, f64)>) -> (u64, u64) {
        let max = f64::from(content_width_twips(self.options.page)) / 20.0 * EMU_PER_PT;
        let (w, h) = match points {
            Some((w, h)) if w > 0.0 && h > 0.0 => (w * EMU_PER_PT, h * EMU_PER_PT),
            _ => match (asset.width, asset.height) {
                (Some(w), Some(h)) if w > 0 && h > 0 => {
                    (f64::from(w) * EMU_PER_PX, f64::from(h) * EMU_PER_PX)
                }
                _ => (max, max * 0.5625),
            },
        };
        let scale = if w > max { max / w } else { 1.0 };
        (emu(w * scale), emu(h * scale))
    }

    fn drawing(&mut self, asset_name: &str, alt: &str, points: Option<(f64, f64)>) -> Option<String> {
        let asset = self.asset(asset_name)?;
        let (png_rel, svg_rel) = self.image_rel(asset)?;
        let (cx, cy) = self.extent(asset, points);
        let id = self.next_drawing;
        self.next_drawing += 1;
        let svg_ext = svg_rel.map_or_else(String::new, |rel| {
            format!(
                r#"<a:extLst><a:ext uri="{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}"><asvg:svgBlip r:embed="{rel}"/></a:ext></a:extLst>"#
            )
        });
        Some(format!(
            concat!(
                r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">"#,
                r#"<wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="Picture {id}" descr="{alt}"/>"#,
                r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>"#,
                r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
                r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="Picture {id}" descr="{alt}"/><pic:cNvPicPr/></pic:nvPicPr>"#,
                r#"<pic:blipFill><a:blip r:embed="{png}">{svg}</a:blip><a:stretch><a:fillRect/></a:stretch></pic:blipFill>"#,
                r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>"#,
                r#"</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
            ),
            cx = cx,
            cy = cy,
            id = id,
            alt = xml(alt),
            png = png_rel,
            svg = svg_ext
        ))
    }

    fn picture_paragraph(&mut self, asset: &str, alt: &str, points: Option<(f64, f64)>) {
        match self.drawing(asset, alt, points) {
            Some(run) => {
                let _ = write!(
                    self.body,
                    r#"<w:p><w:pPr><w:keepNext/><w:jc w:val="center"/></w:pPr>{run}</w:p>"#
                );
            }
            None => self.styled_paragraph(
                "Caption",
                &[Span::plain(format!("[Image: {alt}]"))],
            ),
        }
    }

    // ---- runs ---------------------------------------------------------

    fn runs(&mut self, spans: &[Span], out: &mut String) {
        for span in spans {
            match span {
                Span::Text { text, style } => text_run(text, *style, None, out),
                Span::Break => out.push_str("<w:r><w:br/></w:r>"),
                Span::Link { url, spans } => self.link_runs(url, spans, out),
                Span::Xref { anchor, text } => {
                    let name = self.bookmarks.get(anchor).cloned().unwrap_or_default();
                    let _ = write!(
                        out,
                        r#"<w:hyperlink w:anchor="{}" w:history="1">"#,
                        xml(&name)
                    );
                    text_run(text, Style::default(), Some("Hyperlink"), out);
                    out.push_str("</w:hyperlink>");
                    text_run(", p.\u{a0}", Style::default(), None, out);
                    let _ = write!(
                        out,
                        concat!(
                            r#"<w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r>"#,
                            r#"<w:r><w:instrText xml:space="preserve"> PAGEREF {} \h </w:instrText></w:r>"#,
                            r#"<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>#</w:t></w:r>"#,
                            r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
                        ),
                        xml(&name)
                    );
                }
                Span::Image { asset, alt } => {
                    match self.drawing(asset, alt, Some((12.0, 12.0))) {
                        Some(run) => out.push_str(&run),
                        None => text_run(alt, Style::default(), None, out),
                    }
                }
            }
        }
    }

    fn link_runs(&mut self, url: &str, spans: &[Span], out: &mut String) {
        let rel = self.hyperlink_rel(url);
        let _ = write!(out, r#"<w:hyperlink r:id="{rel}" w:history="1">"#);
        for span in spans {
            match span {
                Span::Text { text, style } => text_run(text, *style, Some("Hyperlink"), out),
                Span::Break => out.push_str("<w:r><w:br/></w:r>"),
                other => {
                    let text = plain_text(std::slice::from_ref(other));
                    text_run(&text, Style::default(), Some("Hyperlink"), out);
                }
            }
        }
        out.push_str("</w:hyperlink>");
    }
}

fn embolden(span: &Span) -> Span {
    match span {
        Span::Text { text, style } => Span::Text {
            text: text.clone(),
            style: Style {
                bold: true,
                ..*style
            },
        },
        other => other.clone(),
    }
}

fn text_run(text: &str, style: Style, char_style: Option<&str>, out: &mut String) {
    if text.is_empty() {
        return;
    }
    out.push_str("<w:r>");
    let mut props = String::new();
    let char_style = if style.code { Some("CodeChar") } else { char_style };
    if let Some(cs) = char_style {
        let _ = write!(props, r#"<w:rStyle w:val="{cs}"/>"#);
    }
    if style.bold {
        props.push_str("<w:b/>");
    }
    if style.italic {
        props.push_str("<w:i/>");
    }
    if style.strike {
        props.push_str("<w:strike/>");
    }
    if style.highlight {
        props.push_str(r#"<w:highlight w:val="yellow"/>"#);
    }
    if style.sub {
        props.push_str(r#"<w:vertAlign w:val="subscript"/>"#);
    } else if style.sup {
        props.push_str(r#"<w:vertAlign w:val="superscript"/>"#);
    }
    if !props.is_empty() {
        let _ = write!(out, "<w:rPr>{props}</w:rPr>");
    }
    let _ = write!(
        out,
        r#"<w:t xml:space="preserve">{}</w:t></w:r>"#,
        xml(text)
    );
}

fn align_ppr(align: Option<Align>) -> &'static str {
    match align {
        Some(Align::Center) => r#"<w:pPr><w:jc w:val="center"/></w:pPr>"#,
        Some(Align::Right) => r#"<w:pPr><w:jc w:val="right"/></w:pPr>"#,
        _ => "",
    }
}

const fn tone_bg(tone: Tone) -> &'static str {
    match tone {
        Tone::Info => "E3ECFA",
        Tone::Warn => "FFF1D6",
        Tone::Good => "E3F4E4",
        Tone::Crit => "FCE4E2",
        Tone::Tip => "EEE8FB",
        Tone::Note => "EEEEF2",
    }
}

const fn tone_fg(tone: Tone) -> &'static str {
    match tone {
        Tone::Info => "2F5DA8",
        Tone::Warn => "9A6200",
        Tone::Good => "2E7D32",
        Tone::Crit => "B3261E",
        Tone::Tip => "6750A4",
        Tone::Note => "5D5E67",
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
fn emu(value: f64) -> u64 {
    if value.is_finite() && value > 0.0 {
        value.min(1e10).round() as u64
    } else {
        0
    }
}

fn collect_headings(nodes: &[Node], depth: u8, out: &mut Vec<(u8, String, Option<String>)>) {
    for node in nodes {
        match node {
            Node::Heading {
                level,
                number,
                spans,
                anchor,
                outlined: true,
            } if *level <= depth => {
                let text = match number {
                    Some(n) => format!("{n}  {}", plain_text(spans)),
                    None => plain_text(spans),
                };
                out.push((*level, text, anchor.clone()));
            }
            Node::Callout { blocks, .. } | Node::Titled { blocks, .. } | Node::Quote { blocks } => {
                collect_headings(blocks, depth, out);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escaping_covers_markup_and_control_characters() {
        assert_eq!(xml("a<b & \"c\" 'd'"), "a&lt;b &amp; &quot;c&quot; &apos;d&apos;");
        assert_eq!(xml("bell\u{7}tab\t"), "belltab\t");
    }

    #[test]
    fn runs_carry_every_style() {
        let mut out = String::new();
        text_run(
            "x",
            Style {
                bold: true,
                italic: true,
                strike: true,
                highlight: true,
                sub: true,
                sup: false,
                code: true,
            },
            None,
            &mut out,
        );
        for tag in [
            "CodeChar",
            "<w:b/>",
            "<w:i/>",
            "<w:strike/>",
            "highlight",
            "subscript",
        ] {
            assert!(out.contains(tag), "{tag} missing from {out}");
        }
    }
}
