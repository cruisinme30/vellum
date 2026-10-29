// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rendering of the Vellum Markdown AST to sanitised HTML, charts and
//! diagrams.
//!
//! [`Renderer::render`] turns a parsed [`Document`] into a [`RenderedPage`]:
//! one `<article class="vl-doc">` of HTML that has passed the allow-list
//! sanitiser, plus its outline and any warnings. The HTML carries no colours
//! of its own; it is styled by [`theme::stylesheet`] through `--vl-*` CSS
//! variables, so one render serves every theme.
//!
//! Charts, roadmaps and the other data fences are drawn here as SVG. Mermaid
//! is rendered in-process; `D2`, PlantUML, Graphviz and Excalidraw go to a
//! Kroki server when one is configured, with the last good SVG as the
//! fallback when it is down. Every SVG and MathML island is sanitised on its
//! own before it is spliced into the page.
//!
//! Render-on-write: [`RenderedBlob`] is the page as the blob the store
//! keeps next to the Markdown, keyed by [`blob_key`].

// Drawing code is geometry: `x * w + gap` reads better than `mul_add`, and
// pixel coordinates do not need its precision; short coordinate names
// (`x`, `y`, `w`, `h`) are the clearest ones; and the drawing, sanitising and
// writing functions are long, linear sequences that read top to bottom.
// `match` on an `Option` is kept where both arms render markup.
#![allow(
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::single_match_else,
    clippy::option_if_let_else,
    reason = "see the comment above"
)]

pub mod code;
pub mod embed;
pub mod figure;
pub mod html;
pub mod math;
pub mod model;
pub mod resolve;
pub mod sanitize;
pub mod theme;
mod writer;

use std::collections::HashSet;
use std::time::Instant;

use futures_util::future::join_all;
use sha2::{Digest as _, Sha256};
use vellum_format::{Block, Document, Fence};

use crate::figure::{Engine, Figures};
use crate::resolve::Resolver;
pub use crate::writer::{TocEntry, Warning};

/// Bumped whenever the HTML for the same input changes, so stored renders
/// are rebuilt.
pub const RENDER_VERSION: u32 = 1;

/// Per-render settings.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// A stable key for the document (its object id). Kroki diagrams use it
    /// to remember their last good SVG per position in the page.
    pub doc_key: Option<String>,
}

/// A rendered page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenderedPage {
    /// Sanitised HTML: one `<article class="vl-doc">`.
    pub html: String,
    /// The front matter title, or the first level-1 heading.
    pub title: Option<String>,
    /// The page outline.
    pub toc: Vec<TocEntry>,
    /// Blocks that rendered with a fallback.
    pub warnings: Vec<Warning>,
    /// Diagrams shown from the last good SVG because Kroki was unavailable.
    pub stale_figures: usize,
}

/// A rendered page as a stored blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBlob {
    /// The HTML bytes.
    pub body: Vec<u8>,
    /// A strong `ETag`: `sha256-<hex>` of the body.
    pub etag: String,
    /// Always `text/html; charset=utf-8`.
    pub content_type: &'static str,
    /// The [`RENDER_VERSION`] that produced it.
    pub render_version: u32,
}

impl RenderedPage {
    /// The page as a blob for render-on-write.
    #[must_use]
    pub fn to_blob(&self) -> RenderedBlob {
        let body = self.html.as_bytes().to_vec();
        let etag = format!("sha256-{}", hex(&Sha256::digest(&body)));
        RenderedBlob {
            body,
            etag,
            content_type: "text/html; charset=utf-8",
            render_version: RENDER_VERSION,
        }
    }
}

/// Where the rendered HTML of a source revision is stored: derived from the
/// source's content hash and the render version, so a new renderer never
/// serves an old render.
#[must_use]
pub fn blob_key(source_sha256: &str) -> String {
    format!(
        "render/v{RENDER_VERSION}/f{}/{source_sha256}.html",
        figure::FIGURE_VERSION
    )
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Renders documents. Cheap to clone and safe to share between tasks.
#[derive(Debug, Clone)]
pub struct Renderer {
    figures: Figures,
}

impl Renderer {
    /// A renderer drawing figures with `figures`.
    #[must_use]
    pub const fn new(figures: Figures) -> Self {
        Self { figures }
    }

    /// The figure renderer, for exports that need standalone SVG.
    #[must_use]
    pub const fn figures(&self) -> &Figures {
        &self.figures
    }

    /// Renders `doc`, fetching Kroki diagrams first (in parallel, each
    /// bounded by the configured timeout).
    pub async fn render(
        &self,
        doc: &Document,
        resolver: &dyn Resolver,
        options: &RenderOptions,
    ) -> RenderedPage {
        let started = Instant::now();
        let fences = kroki_fences(&doc.blocks);
        let slots: Vec<String> = fences
            .iter()
            .enumerate()
            .map(|(n, f)| slot(options, f, n))
            .collect();
        let results = join_all(fences.iter().zip(&slots).map(|(f, s)| async move {
            (figure::content_key(f), self.figures.kroki(f, Some(s)).await)
        }))
        .await;
        let resolved: writer::Resolved = results.into_iter().collect();
        self.finish(doc, resolver, &resolved, started)
    }

    /// Renders `doc` without network access: Kroki diagrams come from the
    /// cache or show their source.
    #[must_use]
    pub fn render_offline(
        &self,
        doc: &Document,
        resolver: &dyn Resolver,
        options: &RenderOptions,
    ) -> RenderedPage {
        let started = Instant::now();
        let resolved: writer::Resolved = kroki_fences(&doc.blocks)
            .iter()
            .enumerate()
            .map(|(n, f)| {
                (
                    figure::content_key(f),
                    self.figures.kroki_cached(f, Some(&slot(options, f, n))),
                )
            })
            .collect();
        self.finish(doc, resolver, &resolved, started)
    }

    fn finish(
        &self,
        doc: &Document,
        resolver: &dyn Resolver,
        resolved: &writer::Resolved,
        started: Instant,
    ) -> RenderedPage {
        let span = tracing::info_span!("render.page", blocks = doc.blocks.len());
        let _entered = span.enter();
        let mut w = writer::Writer::new(resolver, &self.figures, resolved);
        w.document(&doc.blocks);
        let html = sanitize::page(&w.out, &w.islands);
        let title = doc
            .front_matter
            .as_ref()
            .and_then(|fm| fm.title())
            .map(str::to_owned)
            .or_else(|| w.toc.iter().find(|e| e.level == 1).map(|e| e.text.clone()));
        let page = RenderedPage {
            html,
            title,
            toc: w.toc,
            warnings: w.warnings,
            stale_figures: w.stale,
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        tracing::info!(
            histogram.vellum_render_page_ms = ms,
            monotonic_counter.vellum_render_pages = 1_u64,
            monotonic_counter.vellum_render_warnings = page.warnings.len() as u64,
            monotonic_counter.vellum_render_stale_figures = page.stale_figures as u64,
            bytes = page.html.len(),
            "rendered page"
        );
        page
    }
}

/// The Kroki slot of the `n`th Kroki fence: its `{#id}` or its position.
fn slot(options: &RenderOptions, f: &Fence, n: usize) -> String {
    let doc = options.doc_key.as_deref().unwrap_or("doc");
    match f.attrs.id.as_deref() {
        Some(id) => format!("{doc}#{id}"),
        None => format!("{doc}#kroki-{n}"),
    }
}

/// Every Kroki fence in the document, in order, including nested ones.
fn kroki_fences(blocks: &[Block]) -> Vec<&Fence> {
    fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Fence>, depth: usize) {
        if depth > 32 {
            return;
        }
        for b in blocks {
            match b {
                Block::Fence(f) if matches!(figure::engine(f), Some(Engine::Kroki(_))) => {
                    out.push(f);
                }
                Block::BlockQuote { blocks } => walk(blocks, out, depth + 1),
                Block::List(l) => l.items.iter().for_each(|i| walk(&i.blocks, out, depth + 1)),
                Block::Container(c) => c.sections.iter().for_each(|s| walk(s, out, depth + 1)),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut out, 0);
    // Identical diagrams are fetched once.
    let mut seen = HashSet::new();
    out.retain(|f| seen.insert(figure::content_key(f)));
    out
}
