// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! HTML export: one self-contained file.
//!
//! The flow is written as semantic HTML with a small inline stylesheet in
//! the theme's colours. Images and figures are embedded as `data:` URIs
//! (SVG through `<img>`, so nothing in a figure can run script), there
//! is no script, and every piece of text is escaped. Cross-references
//! become in-page links; the contents list is built from the headings.

use std::fmt::Write as _;
use std::time::Instant;

use crate::flow::{
    Align, Flow, Layout, Node, Span, Style, Table, TableStyle, Tone,
};
use crate::metrics;
use crate::pdf::PrintTheme;

/// Writes `flow` as a standalone HTML document.
#[must_use]
#[tracing::instrument(
    name = "export.html",
    skip_all,
    fields(blocks = flow.blocks.len())
)]
pub fn write(flow: &Flow, theme: &PrintTheme) -> String {
    let started = Instant::now();
    let mut w = Writer {
        flow,
        out: String::with_capacity(16 * 1024),
    };
    w.document(theme);
    metrics::export_done("html", started, Ok(w.out.len()));
    w.out
}

struct Writer<'a> {
    flow: &'a Flow,
    out: String,
}

/// Escapes text for HTML content and attribute values.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn safe_url(url: &str) -> Option<&str> {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:"))
    .then_some(url.trim())
}

const fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Info => "info",
        Tone::Warn => "warn",
        Tone::Good => "good",
        Tone::Crit => "crit",
        Tone::Tip => "tip",
        Tone::Note => "note",
    }
}

impl Writer<'_> {
    fn document(&mut self, theme: &PrintTheme) {
        let title = escape(&self.flow.title);
        let lang = escape(&theme.lang);
        let c = &theme.colors;
        let _ = write!(
            self.out,
            "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n\
             <meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, \
             initial-scale=1\">\n<meta name=\"generator\" content=\"Vellum\">\n\
             <title>{title}</title>\n<style>\n\
             :root{{--text:{};--heading:{};--muted:{};--primary:{};--outline:{}\
             ;--surface:{}}}\n",
            c.text, c.heading, c.muted, c.primary, c.outline, c.surface
        );
        let mut tones: Vec<_> = theme.tones.iter().collect();
        tones.sort_by_key(|(tone, _)| tone_class(**tone));
        for (tone, colors) in tones {
            let _ = writeln!(
                self.out,
                ".t-{0}{{--fg:{1};--bg:{2}}}",
                tone_class(*tone),
                colors.fg,
                colors.bg
            );
        }
        self.out.push_str(STYLE);
        self.out.push_str("</style>\n</head>\n<body>\n<main>\n");
        let show = self.flow.show_title || self.flow.layout == Layout::Book;
        if show {
            let _ = writeln!(
                self.out,
                "<header class=\"title\"><h1>{title}</h1><p class=\"muted\">As \
                    of {}</p></header>",
                escape(&self.flow.as_of)
            );
        }
        let blocks = &self.flow.blocks;
        self.blocks(blocks);
        self.out.push_str("</main>\n</body>\n</html>\n");
    }

    fn blocks(&mut self, blocks: &[Node]) {
        for node in blocks {
            self.node(node);
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per node kind
    fn node(&mut self, node: &Node) {
        match node {
            Node::Heading {
                level,
                number,
                spans,
                anchor,
                ..
            } => {
                let level = (*level).clamp(1, 6);
                let id = anchor
                    .as_ref()
                    .map(|a| format!(" id=\"{}\"", escape(a)))
                    .unwrap_or_default();
                let _ = write!(self.out, "<h{level}{id}>");
                if let Some(number) = number {
                    let _ = write!(
                        self.out,
                        "<span class=\"num\">{}</span> ",
                        escape(number)
                    );
                }
                self.spans(spans);
                let _ = writeln!(self.out, "</h{level}>");
            }
            Node::Paragraph { spans, anchor } => {
                let id = anchor
                    .as_ref()
                    .map(|a| format!(" id=\"{}\"", escape(a)))
                    .unwrap_or_default();
                let _ = write!(self.out, "<p{id}>");
                self.spans(spans);
                self.out.push_str("</p>\n");
            }
            Node::Quote { blocks } => {
                self.out.push_str("<blockquote>\n");
                self.blocks(blocks);
                self.out.push_str("</blockquote>\n");
            }
            Node::List { start, items } => {
                let tasks = items.iter().any(|i| i.check.is_some());
                match start {
                    Some(n) if *n != 1 => {
                        let _ = writeln!(self.out, "<ol start=\"{n}\">");
                    }
                    Some(_) => self.out.push_str("<ol>\n"),
                    None if tasks => {
                        self.out.push_str("<ul class=\"tasks\">\n");
                    }
                    None => self.out.push_str("<ul>\n"),
                }
                for item in items {
                    self.out.push_str("<li>");
                    if let Some(done) = item.check {
                        self.out.push_str(if done { "☑ " } else { "☐ " });
                    }
                    // A single paragraph stays inline in the item.
                    match item.blocks.as_slice() {
                        [Node::Paragraph { spans, .. }] => self.spans(spans),
                        blocks => self.blocks(blocks),
                    }
                    self.out.push_str("</li>\n");
                }
                self.out.push_str(if start.is_some() {
                    "</ol>\n"
                } else {
                    "</ul>\n"
                });
            }
            Node::Code { lang, text } => {
                let class = if lang.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"language-{}\"", escape(lang))
                };
                let _ = writeln!(
                    self.out,
                    "<pre><code{class}>{}</code></pre>",
                    escape(text)
                );
            }
            Node::Math { tex, .. } => {
                let _ = writeln!(
                    self.out,
                    "<pre class=\"math\"><code \
                        class=\"language-latex\">{}</code></pre>",
                    escape(tex)
                );
            }
            Node::Table(table) => self.table(table),
            Node::Rule => self.out.push_str("<hr>\n"),
            Node::Callout { tone, blocks } => {
                let _ = writeln!(
                    self.out,
                    "<aside class=\"callout t-{}\"><strong>{}</strong>",
                    tone_class(*tone),
                    escape(tone.label())
                );
                self.blocks(blocks);
                self.out.push_str("</aside>\n");
            }
            Node::Titled { title, blocks } => {
                self.out.push_str("<section class=\"titled\"><h4>");
                self.spans(title);
                self.out.push_str("</h4>\n");
                self.blocks(blocks);
                self.out.push_str("</section>\n");
            }
            Node::Columns { columns } => {
                self.out.push_str("<div class=\"columns\">\n");
                for column in columns {
                    self.out.push_str("<div>\n");
                    self.blocks(column);
                    self.out.push_str("</div>\n");
                }
                self.out.push_str("</div>\n");
            }
            Node::Stats { tiles } => {
                self.out.push_str("<div class=\"tiles\">\n");
                for tile in tiles {
                    let tone = tile
                        .trend
                        .map(|t| format!(" t-{}", tone_class(t)))
                        .unwrap_or_default();
                    let _ = writeln!(
                        self.out,
                        "<div class=\"tile\"><div class=\"value\">{}</div><div \
                            class=\"muted\">{}</div><div \
                            class=\"delta{tone}\">{}</div></div>",
                        escape(&tile.value),
                        escape(&tile.label),
                        escape(&tile.delta)
                    );
                }
                self.out.push_str("</div>\n");
            }
            Node::Cards { cards } => {
                self.out.push_str("<div class=\"tiles\">\n");
                for card in cards {
                    let _ = write!(
                        self.out,
                        "<div class=\"tile\"><div><strong>{} \
                            {}</strong></div><div>",
                        escape(&card.icon),
                        escape(&card.title)
                    );
                    self.spans(&card.spans);
                    self.out.push_str("</div></div>\n");
                }
                self.out.push_str("</div>\n");
            }
            Node::Progress { bars } => {
                for bar in bars {
                    let _ = writeln!(
                        self.out,
                        "<div class=\"bar\"><span>{}</span><meter min=\"0\" \
                            max=\"100\" \
                            value=\"{:.0}\">{:.0}%</meter><span>{:.0}%</span></\
                            div>",
                        escape(&bar.label),
                        bar.percent,
                        bar.percent,
                        bar.percent
                    );
                }
            }
            Node::LinkCard { url, title, text } => {
                let title = escape(title);
                let link = safe_url(url).map_or_else(
                    || title.clone(),
                    |u| format!("<a href=\"{}\">{title}</a>", escape(u)),
                );
                let _ = writeln!(
                    self.out,
                    "<div \
                        class=\"linkcard\"><strong>{link}</strong><p>{}</p><p \
                        class=\"muted\">{}</p></div>",
                    escape(text),
                    escape(url)
                );
            }
            Node::Figure {
                asset,
                alt,
                caption,
                width,
                ..
            } => {
                let src = self.data_uri(asset);
                let _ = write!(
                    self.out,
                    "<figure><img src=\"{src}\" alt=\"{}\" \
                        style=\"max-width:{:.0}pt\">",
                    escape(alt),
                    width
                );
                if !caption.is_empty() {
                    let _ = write!(
                        self.out,
                        "<figcaption>{}</figcaption>",
                        escape(caption)
                    );
                }
                self.out.push_str("</figure>\n");
            }
            Node::Image {
                asset,
                alt,
                caption,
            } => {
                let src = self.data_uri(asset);
                let _ = write!(
                    self.out,
                    "<figure><img src=\"{src}\" alt=\"{}\">",
                    escape(alt)
                );
                if !caption.is_empty() {
                    let _ = write!(
                        self.out,
                        "<figcaption>{}</figcaption>",
                        escape(caption)
                    );
                }
                self.out.push_str("</figure>\n");
            }
            Node::Toc { depth } => self.toc(*depth),
            Node::PageBreak => {
                self.out.push_str("<div class=\"page-break\"></div>\n");
            }
        }
    }

    fn table(&mut self, table: &Table) {
        let class = match table.style {
            TableStyle::Data => "data",
            TableStyle::Facts => "facts",
        };
        let _ = writeln!(self.out, "<table class=\"{class}\">");
        let align = |i: usize| match table.align.get(i) {
            Some(Align::Center) => " style=\"text-align:center\"",
            Some(Align::Right) => " style=\"text-align:right\"",
            _ => "",
        };
        if !table.header.is_empty()
            && table.header.iter().any(|c| !c.spans.is_empty())
        {
            self.out.push_str("<thead><tr>");
            for (i, cell) in table.header.iter().enumerate() {
                let _ = write!(self.out, "<th{}>", align(i));
                self.spans(&cell.spans);
                self.out.push_str("</th>");
            }
            self.out.push_str("</tr></thead>\n");
        }
        self.out.push_str("<tbody>\n");
        for row in &table.rows {
            self.out.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                let _ = write!(self.out, "<td{}>", align(i));
                if let Some(tone) = cell.shade {
                    let _ = write!(
                        self.out,
                        "<span class=\"chip t-{}\">",
                        tone_class(tone)
                    );
                    self.spans(&cell.spans);
                    self.out.push_str("</span>");
                } else {
                    self.spans(&cell.spans);
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>\n");
        }
        self.out.push_str("</tbody></table>\n");
    }

    fn toc(&mut self, depth: u8) {
        let mut entries = Vec::new();
        for node in &self.flow.blocks {
            node.walk(&mut |n| {
                if let Node::Heading {
                    level,
                    number,
                    spans,
                    anchor: Some(anchor),
                    outlined: true,
                } = n
                    && *level <= depth
                {
                    let text = crate::flow::plain_text(spans);
                    let text = number.as_ref().map_or_else(
                        || text.clone(),
                        |n| format!("{n} {text}"),
                    );
                    entries.push((*level, anchor.clone(), text));
                }
            });
        }
        self.out
            .push_str("<nav class=\"toc\"><h2>Contents</h2><ul>\n");
        for (level, anchor, text) in entries {
            let _ = writeln!(
                self.out,
                "<li class=\"l{level}\"><a href=\"#{}\">{}</a></li>",
                escape(&anchor),
                escape(&text)
            );
        }
        self.out.push_str("</ul></nav>\n");
    }

    fn spans(&mut self, spans: &[Span]) {
        for span in spans {
            match span {
                Span::Text { text, style } => self.text(text, *style),
                Span::Link { url, spans } => match safe_url(url) {
                    Some(url) => {
                        let _ =
                            write!(self.out, "<a href=\"{}\">", escape(url));
                        self.spans(spans);
                        self.out.push_str("</a>");
                    }
                    None => self.spans(spans),
                },
                Span::Xref { anchor, text } => {
                    let _ = write!(
                        self.out,
                        "<a class=\"xref\" href=\"#{}\">{}</a>",
                        escape(anchor),
                        escape(text)
                    );
                }
                Span::Image { asset, alt } => {
                    let src = self.data_uri(asset);
                    let _ = write!(
                        self.out,
                        "<img class=\"inline\" src=\"{src}\" alt=\"{}\">",
                        escape(alt)
                    );
                }
                Span::Break => self.out.push_str("<br>"),
            }
        }
    }

    fn text(&mut self, text: &str, style: Style) {
        let tags: [(bool, &str); 7] = [
            (style.code, "code"),
            (style.bold, "strong"),
            (style.italic, "em"),
            (style.strike, "s"),
            (style.highlight, "mark"),
            (style.sub, "sub"),
            (style.sup, "sup"),
        ];
        for (on, tag) in tags {
            if on {
                let _ = write!(self.out, "<{tag}>");
            }
        }
        self.out.push_str(&escape(text));
        for (on, tag) in tags.iter().rev() {
            if *on {
                let _ = write!(self.out, "</{tag}>");
            }
        }
    }

    fn data_uri(&self, asset: &str) -> String {
        self.flow.asset(asset).map_or_else(String::new, |a| {
            format!("data:{};base64,{}", a.media.mime(), base64(&a.bytes))
        })
    }
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n =
            (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (n >> (18 - 6 * i)) & 63;
                out.push(char::from(
                    TABLE.get(index as usize).copied().unwrap_or(b'A'),
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

const STYLE: &str = "\
body{margin:0;color:var(--text);background:#fff;font:16px/1.6 \
    system-ui,-apple-system,'Segoe UI',Roboto,sans-serif}
main{max-width:46rem;margin:0 auto;padding:2rem 1rem}
h1,h2,h3,h4,h5,h6{color:var(--heading);line-height:1.25}
.muted{color:var(--muted)}
a{color:var(--primary)}
pre{background:var(--surface);padding:.75rem \
    1rem;border-radius:6px;overflow-x:auto}
code{font-family:ui-monospace,'Roboto Mono',Menlo,monospace;font-size:.92em}
blockquote{margin:0;padding-left:1rem;border-left:3px solid \
    var(--outline);color:var(--muted)}
table{border-collapse:collapse;width:100%;margin:1rem 0}
th,td{border-bottom:1px solid var(--outline);padding:.4rem \
    .6rem;text-align:left;vertical-align:top}
table.facts td:first-child{font-weight:600;width:30%}
.chip{background:var(--bg);color:var(--fg);border-radius:4px;padding:0 .4rem}
.callout{background:var(--bg);border-left:4px solid var(--fg);padding:.5rem \
    1rem;margin:1rem 0;border-radius:4px}
.columns,.tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(12rem\
    ,1fr));gap:1rem}
.tile,.linkcard{border:1px solid \
    var(--outline);border-radius:8px;padding:.75rem 1rem}
.tile .value{font-size:1.6rem;font-weight:600}
.delta{color:var(--fg,var(--muted))}
.bar{display:grid;grid-template-columns:1fr 2fr \
    3rem;gap:.5rem;align-items:center}
.bar meter{width:100%}
figure{margin:1rem 0;text-align:center}
figure img{max-width:100%;height:auto}
figcaption{color:var(--muted);font-size:.9em}
img.inline{height:1em;vertical-align:middle}
.toc ul{list-style:none;padding-left:0}
.toc .l2{padding-left:1rem}.toc .l3{padding-left:2rem}.toc \
    .l4{padding-left:3rem}
.num{color:var(--muted)}
ul.tasks{list-style:none;padding-left:0}
@media print{.page-break{break-after:page}}
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{Asset, Cell, Media};

    fn flow(blocks: Vec<Node>) -> Flow {
        Flow {
            title: "Gateway <beta>".into(),
            as_of: "2026-10-29".into(),
            layout: Layout::Document,
            show_title: true,
            blocks,
            assets: vec![Asset {
                name: "a0.svg".into(),
                media: Media::Svg,
                bytes: b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
                width: None,
                height: None,
            }],
        }
    }

    #[test]
    fn text_is_escaped_and_unsafe_links_dropped() {
        let html = write(
            &flow(vec![Node::Paragraph {
                spans: vec![
                    Span::plain("<script>alert(1)</script>"),
                    Span::Link {
                        url: "javascript:alert(1)".into(),
                        spans: vec![Span::plain("click")],
                    },
                    Span::Link {
                        url: "https://northstar.dev?a=1&b=2".into(),
                        spans: vec![Span::bold("site")],
                    },
                ],
                anchor: None,
            }]),
            &PrintTheme::default(),
        );
        assert!(html.contains("<title>Gateway &lt;beta&gt;</title>"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("javascript:"));
        assert!(html.contains(
            "<a href=\"https://northstar.dev?a=1&amp;b=2\"><strong>site</st\
                    rong></a>"
        ));
    }

    #[test]
    fn figures_embed_as_images_and_tables_keep_chips() {
        let html = write(
            &flow(vec![
                Node::Figure {
                    asset: "a0.svg".into(),
                    alt: "Chart".into(),
                    caption: "Blocked".into(),
                    width: 300.0,
                    height: 150.0,
                },
                Node::Table(Table {
                    style: TableStyle::Data,
                    align: vec![Align::Left],
                    header: vec![Cell::text("Status")],
                    rows: vec![vec![Cell {
                        spans: vec![Span::plain("Done")],
                        shade: Some(Tone::Good),
                    }]],
                }),
            ]),
            &PrintTheme::default(),
        );
        assert!(
            html.contains("<img src=\"data:image/svg+xml;base64,"),
            "{html}"
        );
        assert!(html.contains("<figcaption>Blocked</figcaption>"));
        assert!(html.contains("<span class=\"chip t-good\">Done</span>"));
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
