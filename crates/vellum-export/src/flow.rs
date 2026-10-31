// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The export model: a document as print and office formats see it.
//!
//! A [`Flow`] has no containers or data fences left. Each block of the
//! dialect has already been mapped to the shape the PRD block catalogue
//! gives it in exports (a callout box, a facts table, a figure, a dated
//! list), live data has been frozen, and every image or figure is an
//! [`Asset`] carried next to the blocks. The PDF and Word backends only
//! have to draw these nodes.
//!
//! The model is serialised to JSON for the Typst template, so the field
//! names below are part of the template contract.

use serde::Serialize;

/// A document lowered for export.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Flow {
    /// The document title, shown on the first page and in metadata.
    pub title: String,
    /// The date live blocks were frozen at, `YYYY-MM-DD`.
    pub as_of: String,
    /// Whether the first page shows a title block (single documents) or a
    /// title page with contents (books).
    pub layout: Layout,
    /// Whether to print the title above the content. Off when the
    /// document already starts with the same level-1 heading.
    pub show_title: bool,
    /// The blocks, in order.
    pub blocks: Vec<Node>,
    /// Images and figures referenced by name from the blocks.
    #[serde(skip)]
    pub assets: Vec<Asset>,
}

impl Flow {
    /// The asset called `name`.
    #[must_use]
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// How the start of the document is laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// A single page: the title as a heading above the content.
    #[default]
    Document,
    /// A book: a title page, then the regenerated contents.
    Book,
}

/// An image or figure carried with the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The name blocks refer to: `a<n>` plus an extension.
    pub name: String,
    /// The media type.
    pub media: Media,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// Intrinsic width in CSS pixels, when known.
    pub width: Option<u32>,
    /// Intrinsic height in CSS pixels, when known.
    pub height: Option<u32>,
}

/// The image formats every backend accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Media {
    /// `image/png`
    Png,
    /// `image/jpeg`
    Jpeg,
    /// `image/gif`
    Gif,
    /// `image/svg+xml`
    Svg,
}

impl Media {
    /// The file extension, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Svg => "svg",
        }
    }

    /// The IANA media type.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Svg => "image/svg+xml",
        }
    }

    /// Sniffs the format from the first bytes; the declared type of an
    /// upload is never trusted.
    #[must_use]
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(Self::Gif)
        } else {
            let head = bytes.get(..bytes.len().min(512)).unwrap_or_default();
            let head = String::from_utf8_lossy(head);
            let head = head.trim_start_matches('\u{feff}').trim_start();
            (head.starts_with("<svg")
                || head.starts_with("<?xml") && head.contains("<svg"))
            .then_some(Self::Svg)
        }
    }
}

/// A block of the export model.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Node {
    /// A heading. `number` is the computed section number in a book.
    Heading {
        /// Level 1 to 6, after any book offset.
        level: u8,
        /// Section number such as `2.3`, shown before the text.
        #[serde(skip_serializing_if = "Option::is_none")]
        number: Option<String>,
        /// The heading text.
        spans: Vec<Span>,
        /// Anchor for cross-references and bookmarks.
        #[serde(skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
        /// Whether the heading appears in the contents and the PDF outline.
        outlined: bool,
    },
    /// A paragraph.
    Paragraph {
        /// The text.
        spans: Vec<Span>,
        /// Anchor of a `^blk-` block ID, for block cross-references.
        #[serde(skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
    },
    /// An indented quote.
    Quote {
        /// The quoted blocks.
        blocks: Vec<Self>,
    },
    /// A bullet or numbered list. Task items carry `check`.
    List {
        /// `None` for bullets, `Some(n)` for a numbered list from `n`.
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        /// The items.
        items: Vec<Item>,
    },
    /// Source code in a monospace block.
    Code {
        /// Language for highlighting; may be empty.
        lang: String,
        /// The code.
        text: String,
    },
    /// Display maths.
    Math {
        /// The `TeX` source, as written.
        tex: String,
        /// The same formula in Typst maths syntax, when it converts.
        #[serde(skip_serializing_if = "Option::is_none")]
        typst: Option<String>,
    },
    /// A table.
    Table(Table),
    /// A horizontal rule.
    Rule,
    /// A shaded callout box.
    Callout {
        /// The tone: info, warn, good, crit, tip or note.
        tone: Tone,
        /// The blocks inside.
        blocks: Vec<Self>,
    },
    /// A titled group: an expanded toggle or one tab of a tab set.
    Titled {
        /// The title, shown as a subheading.
        title: Vec<Span>,
        /// The blocks inside.
        blocks: Vec<Self>,
    },
    /// Side-by-side columns (a borderless table in Word).
    Columns {
        /// One block list per column.
        columns: Vec<Vec<Self>>,
    },
    /// A row of stat tiles (a shaded table in Word).
    Stats {
        /// The tiles.
        tiles: Vec<Stat>,
    },
    /// A grid of cards (a table in Word).
    Cards {
        /// The cards.
        cards: Vec<Card>,
    },
    /// Progress bars (label and percentage in Word).
    Progress {
        /// The bars.
        bars: Vec<Bar>,
    },
    /// A titled link.
    LinkCard {
        /// The target URL.
        url: String,
        /// The title.
        title: String,
        /// A line of description; may be empty.
        text: String,
    },
    /// A chart or diagram drawn as a vector.
    Figure {
        /// The SVG asset name.
        asset: String,
        /// Text alternative.
        alt: String,
        /// Caption below the figure; may be empty.
        caption: String,
        /// Width in points.
        width: f64,
        /// Height in points.
        height: f64,
    },
    /// An image block.
    Image {
        /// The asset name.
        asset: String,
        /// Text alternative.
        alt: String,
        /// Caption below the image; may be empty.
        caption: String,
    },
    /// The regenerated table of contents.
    Toc {
        /// Deepest heading level listed.
        depth: u8,
    },
    /// Start the next block on a new page.
    PageBreak,
}

/// A list item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    /// `Some(done)` for a task item, shown as ☑ or ☐.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<bool>,
    /// The item's blocks.
    pub blocks: Vec<Node>,
}

/// A table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    /// How the table is styled.
    pub style: TableStyle,
    /// One alignment per column.
    pub align: Vec<Align>,
    /// The header row; empty for a table without one.
    pub header: Vec<Cell>,
    /// The body rows; every row has as many cells as there are columns.
    pub rows: Vec<Vec<Cell>>,
}

impl Table {
    /// Number of columns.
    #[must_use]
    pub const fn columns(&self) -> usize {
        self.align.len()
    }
}

/// How a table is styled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TableStyle {
    /// A data table with a header row.
    Data,
    /// A two-column key and value table (key facts, calculator results).
    Facts,
}

/// Column alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Start of the line.
    Left,
    /// Centred.
    Center,
    /// End of the line.
    Right,
}

/// A table cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// The cell text.
    pub spans: Vec<Span>,
    /// A status chip shade, when the cell holds a status word.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade: Option<Tone>,
}

impl Cell {
    /// A plain cell with `text`.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            spans: vec![Span::plain(text)],
            shade: None,
        }
    }
}

/// The colour family of a callout, chip or delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Blue: information.
    Info,
    /// Amber: warning.
    Warn,
    /// Green: decision, success.
    Good,
    /// Red: critical.
    Crit,
    /// Violet: tip.
    Tip,
    /// Grey: note.
    Note,
}

impl Tone {
    /// The tone of a callout container name.
    #[must_use]
    pub fn from_callout(name: &str) -> Self {
        match name {
            "info" => Self::Info,
            "warn" => Self::Warn,
            "good" => Self::Good,
            "crit" => Self::Crit,
            "tip" => Self::Tip,
            _ => Self::Note,
        }
    }

    /// The label printed at the top of a callout.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Warn => "Warning",
            Self::Good => "Decision",
            Self::Crit => "Critical",
            Self::Tip => "Tip",
            Self::Note => "Note",
        }
    }
}

/// A stat tile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stat {
    /// The headline value.
    pub value: String,
    /// What it measures.
    pub label: String,
    /// The change, such as `▲ 6 pts`; may be empty.
    pub delta: String,
    /// The colour of the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<Tone>,
}

/// A card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Card {
    /// A leading icon (usually an emoji); may be empty.
    pub icon: String,
    /// The card title.
    pub title: String,
    /// The card text.
    pub spans: Vec<Span>,
}

/// A progress bar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bar {
    /// What the bar measures.
    pub label: String,
    /// Percent complete, 0 to 100.
    pub percent: f64,
}

/// Inline content: a run of text with one style, or a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Span {
    /// Styled text.
    Text {
        /// The text.
        text: String,
        /// Its style.
        #[serde(flatten)]
        style: Style,
    },
    /// A link to a URL.
    Link {
        /// The destination.
        url: String,
        /// The link text.
        spans: Vec<Self>,
    },
    /// A cross-reference to a heading or block in the same export. Print
    /// formats follow `text` with the page number.
    Xref {
        /// The target anchor.
        anchor: String,
        /// The reference text, such as `§2.3 Rollback`.
        text: String,
    },
    /// An inline image.
    Image {
        /// The asset name.
        asset: String,
        /// Text alternative.
        alt: String,
    },
    /// A line break.
    Break,
}

impl Span {
    /// Unstyled text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            style: Style::default(),
        }
    }

    /// Bold text.
    pub fn bold(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            style: Style {
                bold: true,
                ..Style::default()
            },
        }
    }
}

/// The plain text of spans, for alt text, Word bookmarks and tests.
#[must_use]
pub fn plain_text(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Text { text, .. } | Span::Xref { text, .. } => {
                out.push_str(text);
            }
            Span::Link { spans, .. } => out.push_str(&plain_text(spans)),
            Span::Image { alt, .. } => out.push_str(alt),
            Span::Break => out.push(' '),
        }
    }
    out
}

/// Character styles. Several can apply at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[allow(clippy::struct_excessive_bools)] // one flag per independent style
pub struct Style {
    /// Bold.
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    /// Italic.
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    /// Struck through.
    #[serde(skip_serializing_if = "is_false")]
    pub strike: bool,
    /// Highlighted.
    #[serde(skip_serializing_if = "is_false")]
    pub highlight: bool,
    /// Subscript.
    #[serde(skip_serializing_if = "is_false")]
    pub sub: bool,
    /// Superscript.
    #[serde(skip_serializing_if = "is_false")]
    pub sup: bool,
    /// Inline code.
    #[serde(skip_serializing_if = "is_false")]
    pub code: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde passes a reference
const fn is_false(value: &bool) -> bool {
    !*value
}

impl Flow {
    /// Turns cross-references whose target is not in the flow into plain
    /// text, so a stale or filtered reference never breaks a backend.
    pub(crate) fn drop_dangling_xrefs(&mut self) {
        let mut anchors = std::collections::HashSet::new();
        for node in &self.blocks {
            node.walk(&mut |node| {
                if let Node::Heading {
                    anchor: Some(anchor),
                    ..
                }
                | Node::Paragraph {
                    anchor: Some(anchor),
                    ..
                } = node
                {
                    let _ = anchors.insert(anchor.clone());
                }
            });
        }
        for node in &mut self.blocks {
            node.spans_mut(&mut |spans| {
                for span in spans.iter_mut() {
                    if let Span::Xref { anchor, text } = span
                        && !anchors.contains(anchor.as_str())
                    {
                        *span = Span::plain(std::mem::take(text));
                    }
                }
            });
        }
    }
}

impl Node {
    /// Calls `f` on this node and every node inside it.
    pub fn walk(&self, f: &mut impl FnMut(&Self)) {
        f(self);
        match self {
            Self::Quote { blocks }
            | Self::Callout { blocks, .. }
            | Self::Titled { blocks, .. } => {
                for b in blocks {
                    b.walk(f);
                }
            }
            Self::List { items, .. } => {
                items.iter().flat_map(|i| &i.blocks).for_each(|b| b.walk(f));
            }
            Self::Columns { columns } => {
                columns.iter().flatten().for_each(|b| b.walk(f));
            }
            _ => {}
        }
    }

    /// Calls `f` on every run of spans in this node and the nodes inside
    /// it, including link text.
    pub fn spans_mut(&mut self, f: &mut impl FnMut(&mut Vec<Span>)) {
        fn runs(spans: &mut Vec<Span>, f: &mut impl FnMut(&mut Vec<Span>)) {
            for span in spans.iter_mut() {
                if let Span::Link { spans, .. } = span {
                    runs(spans, f);
                }
            }
            f(spans);
        }
        match self {
            Self::Heading { spans, .. } | Self::Paragraph { spans, .. } => {
                runs(spans, f);
            }
            Self::Titled { title, blocks } => {
                runs(title, f);
                for b in blocks {
                    b.spans_mut(f);
                }
            }
            Self::Quote { blocks } | Self::Callout { blocks, .. } => {
                for b in blocks {
                    b.spans_mut(f);
                }
            }
            Self::List { items, .. } => items
                .iter_mut()
                .flat_map(|i| &mut i.blocks)
                .for_each(|b| b.spans_mut(f)),
            Self::Columns { columns } => {
                columns.iter_mut().flatten().for_each(|b| b.spans_mut(f));
            }
            Self::Table(table) => table
                .header
                .iter_mut()
                .chain(table.rows.iter_mut().flatten())
                .for_each(|cell| runs(&mut cell.spans, f)),
            Self::Cards { cards } => {
                for card in cards.iter_mut() {
                    runs(&mut card.spans, f);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_images_from_their_bytes() {
        assert_eq!(Media::sniff(b"\x89PNG\r\n\x1a\nrest"), Some(Media::Png));
        assert_eq!(Media::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Media::Jpeg));
        assert_eq!(Media::sniff(b"GIF89a..."), Some(Media::Gif));
        assert_eq!(Media::sniff(b"  <svg xmlns='x'/>"), Some(Media::Svg));
        assert_eq!(
            Media::sniff(b"<?xml version='1.0'?><svg/>"),
            Some(Media::Svg)
        );
        assert_eq!(Media::sniff(b"<html>"), None);
    }

    #[test]
    fn spans_serialise_flat_for_the_template() -> Result<(), serde_json::Error>
    {
        let span = Span::Text {
            text: "x".into(),
            style: Style {
                bold: true,
                ..Style::default()
            },
        };
        assert_eq!(
            serde_json::to_string(&span)?,
            r#"{"t":"text","text":"x","bold":true}"#
        );
        Ok(())
    }

    #[test]
    fn plain_text_flattens_links_and_breaks() {
        let spans = [
            Span::plain("a"),
            Span::Break,
            Span::Link {
                url: "u".into(),
                spans: vec![Span::bold("b")],
            },
        ];
        assert_eq!(plain_text(&spans), "a b");
    }
}
