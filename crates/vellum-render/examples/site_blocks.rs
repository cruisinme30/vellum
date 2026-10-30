// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Renders the rich blocks of the help books for the project website
//! (ADR 0071, D309).
//!
//! The website builds the help books with mdBook, which knows only
//! `CommonMark`. This tool runs first, over the staged copy of each book: every
//! top-level `:::` container (callouts, cards, steps, tabs, facts and the
//! rest) and every figure fence (Mermaid, `architecture`, charts, timelines
//! and the other data fences) is rendered by this crate, exactly as Vellum
//! renders it in the app, and put back in the page as one line of HTML that
//! mdBook passes through untouched. Everything else stays Markdown, so
//! mdBook still builds its outline and search index from it.
//!
//! Usage: `site_blocks [--css <file>] <markdown-file>...`
//!
//! `--css` writes the renderer's stylesheet, scoped to `.vl-render` and
//! following the site's `data-theme` attribute, so the toggle re-themes
//! every figure.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vellum_format::FenceKind;
use vellum_render::figure::{FigureConfig, Figures};
use vellum_render::resolve::NoResolver;
use vellum_render::{RenderOptions, Renderer, theme};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut css: Option<PathBuf> = None;
    let mut files = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--css" {
            css = args.next().map(PathBuf::from);
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    let mut stderr = std::io::stderr();
    if let Some(path) = css
        && let Err(error) = std::fs::write(&path, site_stylesheet())
    {
        let _ = writeln!(stderr, "site_blocks: {}: {error}", path.display());
        return ExitCode::FAILURE;
    }
    let renderer = match Figures::new(FigureConfig::default()) {
        Ok(figures) => Renderer::new(figures),
        Err(error) => {
            let _ = writeln!(stderr, "site_blocks: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut blocks = 0;
    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "site_blocks: {}: {error}",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        };
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let (page, count) =
            render_page(&renderer, &source, &|src| image_size(dir, src));
        if count > 0 {
            if let Err(error) = std::fs::write(path, page) {
                let _ = writeln!(
                    stderr,
                    "site_blocks: {}: {error}",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
            blocks += count;
        }
    }
    let _ = writeln!(
        stderr,
        "site_blocks: rendered {blocks} block(s) in {} page(s)",
        files.len()
    );
    ExitCode::SUCCESS
}

/// The renderer's stylesheet for the site, plus the few rules that fit the
/// rendered blocks into a help book page.
fn site_stylesheet() -> String {
    let mut css = theme::scoped_stylesheet(".vl-render", "data-theme");
    css.push_str(SITE_CSS);
    css
}

/// Rules for rendered blocks on the website: the renderer's own page
/// stylesheet sets the article's width and type, which the book already does.
const SITE_CSS: &str = "
.vl-render.vl-site-block { margin-block: 1.5em; }
.vl-site-block > .vl-doc { background: transparent; font-family: inherit; \
    font-size: inherit; }
.vl-site-block > .vl-doc > :first-child { margin-block-start: 0; }
.vl-site-block > .vl-doc > :last-child { margin-block-end: 0; }
.vl-render.vl-site-block svg { max-width: 100%; height: auto; }
.vl-render.vl-site-block figure { overflow-x: auto; }
";

/// A part of a page: Markdown left for mdBook, or a block to render.
#[derive(Debug, PartialEq, Eq)]
enum Part {
    Markdown(String),
    Block(String),
}

/// Looks up an image's width and height in pixels from its `src`.
type ImageSize<'a> = &'a dyn Fn(&str) -> Option<(usize, usize)>;

/// Renders the rich blocks of one page. Returns the new page and how many
/// blocks were rendered. `size` gives the size of the page's own images.
fn render_page(
    renderer: &Renderer,
    source: &str,
    size: ImageSize<'_>,
) -> (String, usize) {
    let (front, body) = split_front_matter(source);
    let mut out = String::from(front);
    let mut count = 0;
    for part in split(body) {
        match part {
            Part::Markdown(text) => out.push_str(&text),
            Part::Block(text) => {
                let parsed = vellum_format::parse(&text);
                let page = renderer.render_offline(
                    &parsed.document,
                    &NoResolver,
                    &RenderOptions::default(),
                );
                out.push('\n');
                let html =
                    size_images(&fix_links(&wrap_article(&page.html)), size);
                out.push_str(&one_line(&html));
                out.push_str("\n\n");
                count += 1;
            }
        }
    }
    (out, count)
}

/// Splits off a leading YAML front matter block, which the site build
/// handles on its own.
fn split_front_matter(source: &str) -> (&str, &str) {
    if let Some(rest) = source.strip_prefix("---\n") {
        for end in ["\n---\n", "\n...\n"] {
            if let Some(at) = rest.find(end) {
                let cut = 4 + at + end.len();
                return (
                    source.get(..cut).unwrap_or(""),
                    source.get(cut..).unwrap_or(source),
                );
            }
        }
    }
    ("", source)
}

/// The opening run of a code fence (three or more backticks or tildes after
/// up to three spaces), and the info string after it.
fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|&x| x == c).count();
    (run >= 3).then(|| (c, run, trimmed.get(run..).unwrap_or("").trim()))
}

/// Whether `line` closes a fence opened with `run` `c`s.
fn fence_close(line: &str, c: char, run: usize) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().all(|x| x == c) && t.len() >= run
}

/// A `:::` container's opening line: its colon count when a name follows.
fn container_open(line: &str) -> Option<usize> {
    let t = line.trim_start_matches(' ');
    let colons = t.chars().take_while(|&x| x == ':').count();
    let rest = t.get(colons..).unwrap_or("").trim();
    (colons >= 3 && !rest.is_empty()).then_some(colons)
}

/// A container's closing line: only colons.
fn container_close(line: &str) -> Option<usize> {
    let t = line.trim();
    (t.len() >= 3 && t.chars().all(|x| x == ':')).then_some(t.len())
}

/// Whether a fence's language is a figure the site should draw. Plain code
/// stays with mdBook's highlighter; collections, which need a server, and
/// the diagram languages that need a Kroki server are left as they are.
fn is_figure(info: &str) -> bool {
    let lang = info
        .split(|c: char| c.is_whitespace() || c == '{')
        .next()
        .unwrap_or("");
    match FenceKind::from_lang(lang) {
        FenceKind::Diagram => lang == "mermaid",
        FenceKind::Chart
        | FenceKind::Timeline
        | FenceKind::Roadmap
        | FenceKind::RiskMatrix
        | FenceKind::Board
        | FenceKind::Architecture
        | FenceKind::Calculator
        | FenceKind::Table
        | FenceKind::Image => true,
        FenceKind::Code | FenceKind::Collection => false,
    }
}

/// Where a block to render ends.
#[derive(Clone, Copy)]
enum End {
    /// A figure fence, closed by a run of at least this many of this char.
    Fence(char, usize),
    /// A container, closed by exactly this many colons at depth zero.
    Container(usize),
}

/// Splits a page body into Markdown and blocks to render. Fences that are
/// not figures, including the help books' ```` ```markdown ```` examples,
/// are kept verbatim with everything inside them.
fn split(body: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut markdown = String::new();
    let mut lines = body.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let end = if let Some((c, run, info)) = fence_open(line) {
            if !is_figure(info) {
                // Copy the whole fence through untouched.
                markdown.push_str(line);
                for inner in lines.by_ref() {
                    markdown.push_str(inner);
                    if fence_close(inner, c, run) {
                        break;
                    }
                }
                continue;
            }
            End::Fence(c, run)
        } else if let Some(colons) = container_open(line) {
            End::Container(colons)
        } else {
            markdown.push_str(line);
            continue;
        };
        let mut block = String::from(line);
        // Nested containers of the same colon count open and close in
        // pairs; fences inside a container are skipped, so their lines
        // never count.
        let mut depth = 0usize;
        let mut inner_fence: Option<(char, usize)> = None;
        for inner in lines.by_ref() {
            block.push_str(inner);
            match end {
                End::Fence(c, run) => {
                    if fence_close(inner, c, run) {
                        break;
                    }
                }
                End::Container(colons) => {
                    if let Some((c, run)) = inner_fence {
                        if fence_close(inner, c, run) {
                            inner_fence = None;
                        }
                    } else if let Some((c, run, _)) = fence_open(inner) {
                        inner_fence = Some((c, run));
                    } else if container_open(inner) == Some(colons) {
                        depth += 1;
                    } else if container_close(inner) == Some(colons) {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                }
            }
        }
        if !markdown.is_empty() {
            parts.push(Part::Markdown(std::mem::take(&mut markdown)));
        }
        parts.push(Part::Block(block));
    }
    if !markdown.is_empty() {
        parts.push(Part::Markdown(markdown));
    }
    parts
}

/// The renderer's `<article class="vl-doc">` in a `.vl-render` block,
/// which the site stylesheet themes. The article stays: it carries the
/// renderer's size, density and radius variables.
fn wrap_article(html: &str) -> String {
    format!("<div class=\"vl-render vl-site-block\">{html}</div>")
}

/// Puts rendered HTML on one line: a blank line would end mdBook's HTML
/// block early. Newlines inside `<pre>` become `&#10;`, so code keeps its
/// lines; elsewhere a newline is only white space.
fn one_line(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut pre = 0usize;
    let mut rest = html;
    while let Some(at) = rest.find(['<', '\n']) {
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at..).unwrap_or("");
        if tail.starts_with('\n') {
            out.push_str(if pre > 0 { "&#10;" } else { " " });
            rest = tail.get(1..).unwrap_or("");
            continue;
        }
        if tail.starts_with("<pre") {
            pre += 1;
        } else if tail.starts_with("</pre") {
            pre = pre.saturating_sub(1);
        }
        out.push('<');
        rest = tail.get(1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// The size of a local image next to the page, read from the file's header.
/// Images on other sites, and files the reader cannot size, give `None`.
fn image_size(dir: &Path, src: &str) -> Option<(usize, usize)> {
    if src.starts_with('/') || src.contains(':') {
        return None;
    }
    let path = src.split(['#', '?']).next().unwrap_or(src);
    let found = imagesize::size(dir.join(path)).ok()?;
    Some((found.width, found.height))
}

/// Gives every image in rendered HTML its size, so the browser keeps its
/// space from first paint instead of pushing the page down when the image
/// arrives (a layout shift). An image with no width gets its `width` and
/// `height`; one the author sized (`width="80%"`) keeps that width and gets
/// the image's aspect ratio, so its height is known too.
fn size_images(html: &str, size: ImageSize<'_>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<img ") {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let end = tail.find('>').map_or(tail.len(), |e| e + 1);
        let (tag, after) = tail.split_at(end);
        rest = after;
        let src = tag
            .split_once(" src=\"")
            .and_then(|(_, s)| s.split_once('"'))
            .map(|(s, _)| s.replace("&amp;", "&"));
        let found = src.as_deref().and_then(size);
        match found {
            Some((w, h)) if w > 0 && h > 0 && !tag.contains(" height=") => {
                let extra = if tag.contains(" width=") {
                    format!(" style=\"aspect-ratio: {w} / {h}\"")
                } else {
                    format!(" width=\"{w}\" height=\"{h}\"")
                };
                out.push_str("<img");
                out.push_str(&extra);
                out.push_str(tag.get(4..).unwrap_or(""));
            }
            _ => out.push_str(tag),
        }
    }
    out.push_str(rest);
    out
}

/// Rewrites relative links for the published books, as build-site.sh does
/// for Markdown links: `<name>.book/` becomes `<name>/`, and a page's
/// `.md` becomes `.html` (`README.md` becomes `index.html`).
fn fix_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("href=\"") {
        let (head, tail) = rest.split_at(at + 6);
        out.push_str(head);
        let end = tail.find('"').unwrap_or(tail.len());
        let (target, after) = tail.split_at(end);
        out.push_str(&fix_target(target));
        rest = after;
    }
    out.push_str(rest);
    out
}

fn fix_target(target: &str) -> String {
    let external = target.starts_with('#')
        || target.starts_with('/')
        || target.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        });
    if external {
        return target.to_owned();
    }
    let (path, fragment) = target
        .split_once('#')
        .map_or((target, None), |(p, f)| (p, Some(f)));
    let mut path = path.replace(".book/", "/");
    if path == "README.md" || path.ends_with("/README.md") {
        path.truncate(path.len() - "README.md".len());
        path.push_str("index.html");
    } else if let Some(stem) = path.strip_suffix(".md") {
        path = format!("{stem}.html");
    }
    let mut out = path;
    if let Some(fragment) = fragment {
        let _ = write!(out, "#{fragment}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Renderer {
        Renderer::new(
            Figures::new(FigureConfig::default())
                .unwrap_or_else(|_| unreachable!()),
        )
    }

    fn kinds(body: &str) -> Vec<&'static str> {
        split(body)
            .iter()
            .map(|p| match p {
                Part::Markdown(_) => "md",
                Part::Block(_) => "block",
            })
            .collect()
    }

    #[test]
    fn containers_and_figures_render_and_examples_stay_source() {
        let body = "Intro\n\n::: tip\nHi\n:::\n\n````markdown\n::: tip\nHi\n\
            :::\n````\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```rust\n\
            fn main() {}\n```\n";
        assert_eq!(kinds(body), ["md", "block", "md", "block", "md"]);
    }

    #[test]
    fn nested_containers_close_at_their_own_colon_count() {
        let body = ":::: columns\n::: column\nA\n:::\n::: column\nB\n:::\n\
            ::::\nafter\n";
        let parts = split(body);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.get(1), Some(&Part::Markdown("after\n".into())));
    }

    #[test]
    fn a_fence_inside_a_container_cannot_close_it() {
        let body = "::: details Code\n```text\n:::\n```\n:::\nafter\n";
        assert_eq!(kinds(body), ["block", "md"]);
    }

    #[test]
    fn rendered_html_is_one_line_and_keeps_code_lines() {
        assert_eq!(
            one_line("<p>a\nb</p>\n<pre>x\ny</pre>"),
            "<p>a b</p> <pre>x&#10;y</pre>"
        );
    }

    #[test]
    fn links_follow_the_published_layout() {
        assert_eq!(
            fix_target("../admin.book/deploy/README.md#top"),
            "../admin/deploy/index.html#top"
        );
        assert_eq!(fix_target("crates.md"), "crates.html");
        assert_eq!(
            fix_target("https://example.com/a.md"),
            "https://example.com/a.md"
        );
        assert_eq!(fix_target("#anchor"), "#anchor");
    }

    #[test]
    fn a_page_renders_a_mermaid_diagram_to_svg() {
        let (page, count) = render_page(
            &renderer(),
            "---\ntitle: T\n---\n# T\n\n```mermaid\nflowchart LR\n  A --> B\n\
                ```\n",
            &|_| None,
        );
        assert_eq!(count, 1);
        assert!(page.starts_with("---\ntitle: T\n---\n# T\n"));
        assert!(page.contains("<svg"), "{page}");
        assert!(!page.contains("\n\n<svg"));
    }

    #[test]
    fn images_keep_their_space_before_they_load() {
        let size = |src: &str| {
            (src == "a.jpg" || src == "b.svg").then_some((800, 600))
        };
        assert_eq!(
            size_images("<p><img src=\"a.jpg\" alt=\"A\"></p>", &size),
            "<p><img width=\"800\" height=\"600\" src=\"a.jpg\" alt=\"A\"></p>"
        );
        assert_eq!(
            size_images("<img src=\"b.svg\" width=\"80%\">", &size),
            "<img style=\"aspect-ratio: 800 / 600\" src=\"b.svg\" \
                width=\"80%\">"
        );
        assert_eq!(
            size_images("<img src=\"https://x/c.png\">", &size),
            "<img src=\"https://x/c.png\">"
        );
    }
}
