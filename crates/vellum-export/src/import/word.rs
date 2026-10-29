// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Word (`.docx`) documents, and Google Docs through their Word export.
//!
//! The reader walks `word/document.xml` with the styles, numbering and
//! relationships it refers to: heading and title styles become headings,
//! numbered paragraphs become (nested) lists, quote and code styles
//! become quotes and fenced blocks, tables become GFM tables, runs keep
//! bold, italic, strikethrough, highlight, sub- and superscript and code
//! fonts, hyperlinks keep their targets and pictures become attachments.
//! Tracked deletions, the table of contents and field codes are left
//! out; equations and footnotes come in as text with a warning.

use std::collections::{HashMap, HashSet};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use vellum_format::{Alignment, Attributes, Block, Document, Fence, Inline, List, ListItem, Table};

use super::{Builder, archive};
use crate::error::ImportError;

const FORMAT: &str = "docx";

/// Deepest element nesting read, to bound hostile input.
const MAX_DEPTH: usize = 256;

pub(super) fn import(name: &str, bytes: &[u8]) -> Result<Builder, ImportError> {
    let entries = archive::read(FORMAT, bytes, false)?;
    let file = |path: &str| {
        entries
            .iter()
            .find(|e| e.path == path)
            .map(|e| String::from_utf8_lossy(&e.bytes).into_owned())
    };
    let document = file("word/document.xml")
        .ok_or_else(|| ImportError::format(FORMAT, "word/document.xml is missing"))?;
    let styles = file("word/styles.xml")
        .map(|x| styles(&x))
        .unwrap_or_default();
    let numbering = file("word/numbering.xml")
        .map(|x| numbering(&x))
        .unwrap_or_default();
    let rels = file("word/_rels/document.xml.rels")
        .map(|x| rels(&x))
        .unwrap_or_default();

    let mut builder = Builder::new(FORMAT);
    let read = read_body(&document)
        .map_err(|e| ImportError::format(FORMAT, format!("document.xml: {e}")))?;
    let mut ctx = Convert {
        styles: &styles,
        numbering: &numbering,
        rels: &rels,
        warned: HashSet::new(),
        warnings: Vec::new(),
    };
    let blocks = ctx.blocks(read);
    for warning in ctx.warnings {
        builder.warn(name, warning);
    }
    let stem = super::stem(super::file_name(name)).trim();
    let stem = if stem.is_empty() { "Document" } else { stem };
    let title = blocks
        .iter()
        .find_map(|b| match b {
            Block::Heading { content, .. } => Some(crate::lower::inline_text(content)),
            _ => None,
        })
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| stem.to_owned());
    let text = vellum_format::serialize(&Document {
        front_matter: None,
        blocks,
    });
    let _ = builder.page(&format!("{stem}.md"), &format!("{stem}.md"), &title, text);
    for entry in entries {
        if let Some(media) = entry.path.strip_prefix("word/media/") {
            let out = format!("{stem} assets/{media}");
            builder.asset(&entry.path, &out, entry.bytes);
        }
    }
    Ok(builder)
}

// ---- styles, numbering, relationships ---------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Heading(u8),
    Title,
    Quote,
    Code,
    Toc,
}

#[derive(Default)]
struct RawStyle {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
}

fn styles(xml: &str) -> HashMap<String, Style> {
    let mut raw: HashMap<String, RawStyle> = HashMap::new();
    let mut reader = Reader::from_str(xml);
    let mut current: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => match e.local_name().as_ref() {
                "style" => {
                    current = attr(&e, "styleId");
                    if let Some(id) = &current {
                        let _ = raw.insert(id.clone(), RawStyle::default());
                    }
                }
                "name" | "basedOn" | "outlineLvl" => {
                    if let Some(style) = current.as_ref().and_then(|id| raw.get_mut(id)) {
                        let value = attr(&e, "val").unwrap_or_default();
                        match e.local_name().as_ref() {
                            "name" => style.name = value,
                            "basedOn" => style.based_on = Some(value),
                            _ => style.outline = value.parse().ok(),
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.local_name().as_ref() == "style" => current = None,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    let classify = |style: &RawStyle| -> Option<Style> {
        let name = style.name.to_lowercase();
        if name == "title" {
            return Some(Style::Title);
        }
        if let Some(n) = name
            .strip_prefix("heading ")
            .and_then(|n| n.trim().parse::<u8>().ok())
        {
            return Some(Style::Heading(n.clamp(1, 6)));
        }
        if name.starts_with("toc ") || name == "toc heading" {
            return Some(Style::Toc);
        }
        if name.contains("quote") {
            return Some(Style::Quote);
        }
        if name.contains("code") || name.contains("preformatted") || name.contains("source") {
            return Some(Style::Code);
        }
        style.outline.map(|n| Style::Heading((n + 1).clamp(1, 6)))
    };
    let mut out = HashMap::new();
    for id in raw.keys() {
        let mut current = raw.get(id);
        for _ in 0..10 {
            let Some(style) = current else { break };
            if let Some(kind) = classify(style) {
                let _ = out.insert(id.clone(), kind);
                break;
            }
            current = style.based_on.as_ref().and_then(|b| raw.get(b));
        }
    }
    out
}

/// `numId` → level → ordered start (`None` for bullets).
type Numbering = HashMap<String, HashMap<u8, Option<u64>>>;

fn numbering(xml: &str) -> Numbering {
    let mut abstracts: HashMap<String, HashMap<u8, Option<u64>>> = HashMap::new();
    let mut nums: HashMap<String, String> = HashMap::new();
    let mut reader = Reader::from_str(xml);
    let (mut abstract_id, mut level, mut num_id) = (None::<String>, None::<u8>, None::<String>);
    let mut start = 1u64;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => match e.local_name().as_ref() {
                "abstractNum" => abstract_id = attr(&e, "abstractNumId"),
                "lvl" => {
                    level = attr(&e, "ilvl").and_then(|l| l.parse().ok());
                    start = 1;
                }
                "start" => start = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(1),
                "numFmt" => {
                    if let (Some(a), Some(l)) = (&abstract_id, level) {
                        let ordered = attr(&e, "val").is_some_and(|v| v != "bullet" && v != "none");
                        let _ = abstracts
                            .entry(a.clone())
                            .or_default()
                            .insert(l, ordered.then_some(start));
                    }
                }
                "num" => num_id = attr(&e, "numId"),
                "abstractNumId" => {
                    if let (Some(n), Some(a)) = (&num_id, attr(&e, "val")) {
                        let _ = nums.insert(n.clone(), a);
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "abstractNum" => abstract_id = None,
                "lvl" => level = None,
                "num" => num_id = None,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    nums.into_iter()
        .filter_map(|(num, abs)| Some((num, abstracts.get(&abs)?.clone())))
        .collect()
}

#[derive(Debug, Clone)]
enum Rel {
    Image(String),
    Link(String),
}

fn rels(xml: &str) -> HashMap<String, Rel> {
    let mut out = HashMap::new();
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Relationship" => {
                let (Some(id), Some(kind), Some(target)) =
                    (attr(&e, "Id"), attr(&e, "Type"), attr(&e, "Target"))
                else {
                    continue;
                };
                let rel = if kind.ends_with("/image") {
                    let target = target.trim_start_matches('/');
                    let target = target.strip_prefix("word/").unwrap_or(target);
                    Rel::Image(format!("word/{target}"))
                } else if kind.ends_with("/hyperlink") {
                    Rel::Link(target)
                } else {
                    continue;
                };
                let _ = out.insert(id, rel);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(std::borrow::Cow::into_owned)
        })
}

fn on(e: &BytesStart<'_>) -> bool {
    attr(e, "val").is_none_or(|v| !matches!(v.as_str(), "0" | "false" | "none"))
}

// ---- reading the body -------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // one flag per run property
struct RunStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    highlight: bool,
    sub: bool,
    sup: bool,
    code: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String, RunStyle, Option<String>),
    Break,
    Image { rel: String, alt: String },
    Math(String),
}

#[derive(Debug, Clone, Default)]
struct Para {
    style: Option<String>,
    num: Option<(String, u8)>,
    pieces: Vec<Piece>,
}

#[derive(Debug, Clone)]
enum Item {
    Para(Para),
    Table(Vec<Vec<Vec<Para>>>),
    Footnote,
}

#[derive(Default)]
struct Reading {
    items: Vec<Item>,
    para: Option<Para>,
    run: RunStyle,
    in_rpr: bool,
    in_ppr: bool,
    link: Vec<Option<String>>,
    /// Field nesting: (instruction text, in result part).
    fields: Vec<(String, bool)>,
    deleted: usize,
    math: Option<String>,
    math_depth: usize,
    /// Rows → cells → paragraphs of the outermost open table.
    table: Option<Vec<Vec<Vec<Para>>>>,
    table_depth: usize,
    image_alt: String,
}

#[allow(clippy::too_many_lines)] // one arm per element
fn read_body(xml: &str) -> Result<Vec<Item>, String> {
    let mut reader = Reader::from_str(xml);
    let mut r = Reading::default();
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                if !empty {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err("nesting is too deep".into());
                    }
                }
                let name = e.local_name();
                match name.as_ref() {
                    "p" if r.math_depth == 0 => {
                        r.para = Some(Para::default());
                        if empty {
                            r.end_para();
                        }
                    }
                    "pStyle" => {
                        if let Some(p) = &mut r.para {
                            p.style = attr(e, "val");
                        }
                    }
                    "ilvl" => {
                        if let Some(p) = &mut r.para {
                            let level = attr(e, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                            p.num = Some((p.num.take().map(|n| n.0).unwrap_or_default(), level));
                        }
                    }
                    "numId" => {
                        if let Some(p) = &mut r.para {
                            let level = p.num.as_ref().map_or(0, |n| n.1);
                            p.num = attr(e, "val").filter(|v| v != "0").map(|v| (v, level));
                        }
                    }
                    "r" => r.run = RunStyle::default(),
                    "rPr" => r.in_rpr = true,
                    "pPr" if !empty => r.in_ppr = true,
                    "b" if r.in_rpr => r.run.bold = on(e),
                    "i" if r.in_rpr => r.run.italic = on(e),
                    "strike" | "dstrike" if r.in_rpr => r.run.strike = on(e),
                    "highlight" if r.in_rpr => r.run.highlight = on(e),
                    "vertAlign" if r.in_rpr => match attr(e, "val").as_deref() {
                        Some("superscript") => r.run.sup = true,
                        Some("subscript") => r.run.sub = true,
                        _ => {}
                    },
                    "rFonts" if r.in_rpr => {
                        r.run.code |= attr(e, "ascii").is_some_and(|f| is_mono(&f));
                    }
                    "rStyle" if r.in_rpr => {
                        r.run.code |=
                            attr(e, "val").is_some_and(|s| s.to_lowercase().contains("code"));
                    }
                    "hyperlink" if !empty => {
                        let target = attr(e, "id").map(|id| format!("rel:{id}"));
                        r.link.push(target);
                    }
                    "del" if !empty => r.deleted += 1,
                    "fldChar" => match attr(e, "fldCharType").as_deref() {
                        Some("begin") => r.fields.push((String::new(), false)),
                        Some("separate") => {
                            if let Some(field) = r.fields.last_mut() {
                                field.1 = true;
                            }
                        }
                        Some("end") => {
                            let _ = r.fields.pop();
                        }
                        _ => {}
                    },
                    "tab" if r.para.is_some() && !r.in_ppr => r.text(" "),
                    "br" | "cr" if r.para.is_some() => {
                        if attr(e, "type").as_deref() != Some("page") {
                            r.push(Piece::Break);
                        }
                    }
                    "noBreakHyphen" => r.text("-"),
                    "docPr" => {
                        r.image_alt = attr(e, "descr")
                            .filter(|d| !d.is_empty())
                            .or_else(|| attr(e, "title"))
                            .unwrap_or_default();
                    }
                    "blip" | "imagedata" => {
                        let rel = attr(e, "embed").or_else(|| attr(e, "id"));
                        if let Some(rel) = rel {
                            let alt = std::mem::take(&mut r.image_alt);
                            r.push(Piece::Image { rel, alt });
                        }
                    }
                    "footnoteReference" | "endnoteReference" => r.items.push(Item::Footnote),
                    "oMath" if !empty => {
                        r.math_depth += 1;
                        if r.math_depth == 1 {
                            r.math = Some(String::new());
                        }
                    }
                    "tbl" if !empty => {
                        r.table_depth += 1;
                        if r.table_depth == 1 {
                            r.table = Some(Vec::new());
                        }
                    }
                    "tr" if r.table_depth == 1 => {
                        if let Some(t) = &mut r.table {
                            t.push(Vec::new());
                        }
                    }
                    "tc" if r.table_depth == 1 => {
                        if let Some(row) = r.table.as_mut().and_then(|t| t.last_mut()) {
                            row.push(Vec::new());
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                let text = t.xml10_content().into_owned();
                r.content(&text);
            }
            Event::GeneralRef(g) => {
                let c = g.resolve_char_ref().ok().flatten().or_else(|| {
                    match g.xml10_content().as_ref() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    }
                });
                if let Some(c) = c {
                    r.content(&c.to_string());
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                match e.local_name().as_ref() {
                    "p" if r.math_depth == 0 => r.end_para(),
                    "rPr" => r.in_rpr = false,
                    "pPr" => r.in_ppr = false,
                    "hyperlink" => {
                        let _ = r.link.pop();
                    }
                    "del" => r.deleted = r.deleted.saturating_sub(1),
                    "oMath" => {
                        r.math_depth = r.math_depth.saturating_sub(1);
                        if r.math_depth == 0
                            && let Some(math) = r.math.take()
                        {
                            r.push(Piece::Math(math));
                        }
                    }
                    "tbl" => {
                        r.table_depth = r.table_depth.saturating_sub(1);
                        if r.table_depth == 0
                            && let Some(table) = r.table.take()
                        {
                            r.items.push(Item::Table(table));
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(r.items)
}

impl Reading {
    fn in_instruction(&self) -> bool {
        self.fields.last().is_some_and(|(_, result)| !result)
    }

    fn content(&mut self, text: &str) {
        if self.deleted > 0 {
            return;
        }
        if let Some(math) = &mut self.math {
            math.push_str(text);
            return;
        }
        if self.in_instruction() {
            if let Some((instr, _)) = self.fields.last_mut() {
                instr.push_str(text);
            }
            return;
        }
        self.text(text);
    }

    fn text(&mut self, text: &str) {
        if self.deleted > 0 || self.para.is_none() {
            return;
        }
        let link = self
            .link
            .iter()
            .rev()
            .find_map(Clone::clone)
            .or_else(|| self.field_link());
        let run = self.run;
        self.push(Piece::Text(text.to_owned(), run, link));
    }

    /// The target of an enclosing `HYPERLINK "…"` field.
    fn field_link(&self) -> Option<String> {
        self.fields.iter().rev().find_map(|(instr, result)| {
            let rest = instr.trim().strip_prefix("HYPERLINK")?;
            if !result {
                return None;
            }
            let url = rest.trim().strip_prefix('"')?.split('"').next()?;
            Some(url.to_owned())
        })
    }

    fn push(&mut self, piece: Piece) {
        if let Some(p) = &mut self.para {
            p.pieces.push(piece);
        }
    }

    fn end_para(&mut self) {
        let Some(para) = self.para.take() else {
            return;
        };
        if self.table_depth > 0 {
            if let Some(cell) = self
                .table
                .as_mut()
                .and_then(|t| t.last_mut())
                .and_then(|row| row.last_mut())
            {
                cell.push(para);
            }
        } else {
            self.items.push(Item::Para(para));
        }
    }
}

fn is_mono(font: &str) -> bool {
    let font = font.to_lowercase();
    [
        "courier",
        "consolas",
        "menlo",
        "monaco",
        "mono",
        "source code",
        "fira code",
        "lucida console",
    ]
    .iter()
    .any(|m| font.contains(m))
}

// ---- building blocks --------------------------------------------------

struct Convert<'a> {
    styles: &'a HashMap<String, Style>,
    numbering: &'a Numbering,
    rels: &'a HashMap<String, Rel>,
    warned: HashSet<&'static str>,
    warnings: Vec<String>,
}

impl Convert<'_> {
    fn warn_once(&mut self, key: &'static str, message: &str) {
        if self.warned.insert(key) {
            self.warnings.push(message.to_owned());
        }
    }

    fn style(&self, para: &Para) -> Option<Style> {
        para.style
            .as_ref()
            .and_then(|s| self.styles.get(s))
            .copied()
    }

    fn blocks(&mut self, items: Vec<Item>) -> Vec<Block> {
        let mut out = Vec::new();
        let mut items = items.into_iter().peekable();
        while let Some(item) = items.next() {
            match item {
                Item::Footnote => self.warn_once(
                    "footnote",
                    "footnotes and endnotes were left out; their markers are not imported",
                ),
                Item::Table(rows) => {
                    if let Some(table) = self.table(rows) {
                        out.push(Block::Table(table));
                    }
                }
                Item::Para(para) => match self.style(&para) {
                    Some(Style::Toc) => {}
                    Some(Style::Title) => self.heading(1, &para, &mut out),
                    Some(Style::Heading(level)) => self.heading(level, &para, &mut out),
                    Some(Style::Code) => {
                        let mut literal = plain(&para);
                        literal.push('\n');
                        while let Some(Item::Para(next)) = items.peek() {
                            if self.style(next) != Some(Style::Code) {
                                break;
                            }
                            literal.push_str(&plain(next));
                            literal.push('\n');
                            let _ = items.next();
                        }
                        out.push(Block::Fence(Fence {
                            lang: String::new(),
                            args: String::new(),
                            attrs: Attributes::default(),
                            literal,
                        }));
                    }
                    Some(Style::Quote) => {
                        let mut quoted = self.paragraph(&para);
                        while let Some(Item::Para(next)) = items.peek() {
                            if self.style(next) != Some(Style::Quote) {
                                break;
                            }
                            let next = next.clone();
                            let _ = items.next();
                            quoted.extend(self.paragraph(&next));
                        }
                        out.push(Block::BlockQuote { blocks: quoted });
                    }
                    None if para.num.is_some() => {
                        let mut run = vec![para];
                        while let Some(Item::Para(next)) = items.peek() {
                            if next.num.is_none() || self.style(next).is_some() {
                                break;
                            }
                            let next = next.clone();
                            let _ = items.next();
                            run.push(next);
                        }
                        out.push(self.list(&run));
                    }
                    None => out.extend(self.paragraph(&para)),
                },
            }
        }
        out
    }

    fn heading(&mut self, level: u8, para: &Para, out: &mut Vec<Block>) {
        let content = self.inlines(&para.pieces);
        if !content.is_empty() {
            out.push(Block::Heading {
                level,
                content,
                attrs: Attributes::default(),
                id: None,
            });
        }
    }

    /// A paragraph, or display math when the paragraph is one equation.
    fn paragraph(&mut self, para: &Para) -> Vec<Block> {
        if let [Piece::Math(math)] = para.pieces.as_slice() {
            self.warn_once("math", "equations were imported as their text, not as TeX");
            return vec![Block::Math {
                literal: math.trim().to_owned(),
            }];
        }
        let content = self.inlines(&para.pieces);
        if content.is_empty() {
            Vec::new()
        } else {
            vec![Block::Paragraph { content, id: None }]
        }
    }

    fn list(&mut self, paras: &[Para]) -> Block {
        // A stack of open lists, one per level.
        let mut stack: Vec<(u8, List)> = Vec::new();
        for para in paras {
            let (num, level) = para.num.clone().unwrap_or_default();
            let start = self
                .numbering
                .get(&num)
                .and_then(|levels| levels.get(&level))
                .copied()
                .flatten();
            while stack.len() > 1 && stack.last().is_some_and(|(l, _)| *l > level) {
                close(&mut stack);
            }
            if stack.last().is_none_or(|(l, _)| *l < level) {
                stack.push((
                    level,
                    List {
                        start,
                        tight: true,
                        items: Vec::new(),
                    },
                ));
            }
            let blocks = self.paragraph(para);
            if let Some((_, list)) = stack.last_mut() {
                list.items.push(ListItem { task: None, blocks });
            }
        }
        while stack.len() > 1 {
            close(&mut stack);
        }
        Block::List(stack.pop().map_or_else(
            || List {
                start: None,
                tight: true,
                items: Vec::new(),
            },
            |(_, list)| list,
        ))
    }

    fn table(&mut self, rows: Vec<Vec<Vec<Para>>>) -> Option<Table> {
        let width = rows.iter().map(Vec::len).max().filter(|w| *w > 0)?;
        let mut cells: Vec<Vec<Vec<Inline>>> = rows
            .into_iter()
            .map(|row| {
                let mut row: Vec<Vec<Inline>> = row
                    .iter()
                    .map(|paras| {
                        let mut out = Vec::new();
                        for para in paras {
                            let content = self.inlines(&para.pieces);
                            if content.is_empty() {
                                continue;
                            }
                            if !out.is_empty() {
                                out.push(Inline::HardBreak);
                            }
                            out.extend(content);
                        }
                        out
                    })
                    .collect();
                row.resize(width, Vec::new());
                row
            })
            .collect();
        if cells.is_empty() {
            return None;
        }
        // Header rows are bold already; drop bold that covers a whole cell.
        let header = cells
            .remove(0)
            .into_iter()
            .map(|cell| match <[Inline; 1]>::try_from(cell) {
                Ok([Inline::Strong { content }]) => content,
                Ok(one) => one.to_vec(),
                Err(cell) => cell,
            })
            .collect();
        Some(Table {
            alignments: vec![Alignment::None; width],
            header,
            rows: cells,
        })
    }

    fn inlines(&mut self, pieces: &[Piece]) -> Vec<Inline> {
        // Merge neighbouring runs with the same style and link.
        let mut runs: Vec<Piece> = Vec::new();
        for piece in pieces {
            match (runs.last_mut(), piece) {
                (Some(Piece::Text(text, style, link)), Piece::Text(more, s, l))
                    if style == s && link == l =>
                {
                    text.push_str(more);
                }
                _ => runs.push(piece.clone()),
            }
        }
        let mut out: Vec<Inline> = Vec::new();
        let mut i = 0;
        while let Some(piece) = runs.get(i) {
            match piece {
                Piece::Text(_, _, Some(link)) => {
                    let link = link.clone();
                    let mut content = Vec::new();
                    while let Some(Piece::Text(text, style, Some(l))) = runs.get(i) {
                        if *l != link {
                            break;
                        }
                        content.extend(styled(text, *style));
                        i += 1;
                    }
                    match self.link_target(&link) {
                        Some(url) => out.push(Inline::Link {
                            url,
                            title: String::new(),
                            content,
                        }),
                        None => out.extend(content),
                    }
                    continue;
                }
                Piece::Text(text, style, None) => out.extend(styled(text, *style)),
                Piece::Break => out.push(Inline::HardBreak),
                Piece::Image { rel, alt } => match self.rels.get(rel) {
                    Some(Rel::Image(path)) => out.push(Inline::Image {
                        url: path.clone(),
                        title: String::new(),
                        alt: if alt.is_empty() {
                            Vec::new()
                        } else {
                            vec![Inline::Text { text: alt.clone() }]
                        },
                    }),
                    _ => self.warn_once(
                        "image",
                        "some pictures are not stored in the file and were left out",
                    ),
                },
                Piece::Math(math) => {
                    self.warn_once("math", "equations were imported as their text, not as TeX");
                    out.push(Inline::Code {
                        code: math.trim().to_owned(),
                    });
                }
            }
            i += 1;
        }
        // Trim the paragraph's edges.
        while matches!(out.last(), Some(Inline::HardBreak)) {
            let _ = out.pop();
        }
        if let Some(Inline::Text { text }) = out.first_mut() {
            *text = text.trim_start().to_owned();
        }
        if let Some(Inline::Text { text }) = out.last_mut() {
            *text = text.trim_end().to_owned();
        }
        out.retain(|i| !matches!(i, Inline::Text { text } if text.is_empty()));
        out
    }

    fn link_target(&self, link: &str) -> Option<String> {
        let url = match link.strip_prefix("rel:") {
            Some(rel) => match self.rels.get(rel) {
                Some(Rel::Link(url)) => url.clone(),
                _ => return None,
            },
            None => link.to_owned(),
        };
        let lower = url.trim().to_ascii_lowercase();
        (lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:"))
        .then_some(url)
    }
}

fn close(stack: &mut Vec<(u8, List)>) {
    let Some((_, inner)) = stack.pop() else {
        return;
    };
    if let Some((_, outer)) = stack.last_mut() {
        if let Some(item) = outer.items.last_mut() {
            item.blocks.push(Block::List(inner));
        } else {
            outer.items.push(ListItem {
                task: None,
                blocks: vec![Block::List(inner)],
            });
        }
    }
}

fn plain(para: &Para) -> String {
    para.pieces
        .iter()
        .map(|p| match p {
            Piece::Text(text, ..) | Piece::Math(text) => text.as_str(),
            Piece::Break => "\n",
            Piece::Image { .. } => "",
        })
        .collect()
}

/// One run as inlines, with its spaces kept outside the emphasis so the
/// Markdown stays valid (`**bold** next`, not `**bold **next`).
fn styled(text: &str, style: RunStyle) -> Vec<Inline> {
    let core = text.trim();
    if core.is_empty() || style == RunStyle::default() {
        return vec![Inline::Text {
            text: text.to_owned(),
        }];
    }
    let lead = text.len() - text.trim_start().len();
    let trail = text.len() - text.trim_end().len();
    let mut node = if style.code {
        Inline::Code {
            code: core.to_owned(),
        }
    } else {
        Inline::Text {
            text: core.to_owned(),
        }
    };
    if !style.code {
        let wrap = |on: bool, node: Inline, f: fn(Vec<Inline>) -> Inline| {
            if on { f(vec![node]) } else { node }
        };
        node = wrap(style.strike, node, |content| Inline::Strikethrough {
            content,
        });
        node = wrap(style.italic, node, |content| Inline::Emphasis { content });
        node = wrap(style.bold, node, |content| Inline::Strong { content });
    }
    let wrap = |on: bool, node: Inline, f: fn(Vec<Inline>) -> Inline| {
        if on { f(vec![node]) } else { node }
    };
    node = wrap(style.highlight, node, |content| Inline::Highlight {
        content,
    });
    node = wrap(style.sub, node, |content| Inline::Subscript { content });
    node = wrap(style.sup, node, |content| Inline::Superscript { content });
    let mut out = Vec::with_capacity(3);
    if lead > 0 {
        out.push(Inline::Text { text: " ".into() });
    }
    out.push(node);
    if trail > 0 {
        out.push(Inline::Text { text: " ".into() });
    }
    out
}
