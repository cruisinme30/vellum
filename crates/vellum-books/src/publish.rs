// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Publishing a book as a static site or as an mdBook source tree
//! (R-BK-10).
//!
//! Both outputs start from the same published view of the book:
//!
//! 1. sections the audience may not see are left out entirely, titles
//!    included (a public site never shows a restricted section);
//! 2. front matter is stripped and the title becomes the page's `#`
//!    heading;
//! 3. cross-references become their labels (`§2.3 Services`) and
//!    `vellum://doc/<id>` links become relative links between pages; links
//!    to sections that are left out or outside the book become plain text.
//!
//! The static site renders each page through a [`Renderer`] (the render
//! service in production) and wraps it in navigation with numbers,
//! previous/next links and a stylesheet, with no scripts, so it works
//! under a strict content security policy. The mdBook tree builds with
//! unmodified `mdbook build`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::time::Instant;

use crate::book::Book;
use crate::error::{BookError, Result};
use crate::id::NodeId;
use crate::manifest::Principal;
use crate::markdown::{self, LinkEdit, LinkKind};
use crate::path::{self, ChapterPath};
use crate::summary::{Chapter, Item, Section};
use crate::toc::{Toc, TocEntry, TocItem};
use crate::xref::is_cross_ref_text;

/// Who a publication is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
    /// Anyone on the internet: every restricted section is left out.
    Public,
    /// People holding any of these principals (an internal site).
    Members(BTreeSet<Principal>),
}

/// Turns Vellum Markdown into sanitised HTML. Implemented by the render
/// service; the result is inserted into pages as is, so it must already be
/// sanitised.
pub trait Renderer {
    /// Renders one page body.
    ///
    /// # Errors
    ///
    /// Returns a message when the page cannot be rendered.
    fn render(&self, markdown: &str) -> std::result::Result<String, String>;
}

/// A fallback [`Renderer`] that shows the Markdown source as escaped text
/// in paragraphs. It is safe for any input and is used in tests and when
/// no render service is wired in.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainRenderer;

impl Renderer for PlainRenderer {
    fn render(&self, markdown: &str) -> std::result::Result<String, String> {
        let mut out = String::new();
        for block in markdown
            .split("\n\n")
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            let _ = writeln!(out, "<p>{}</p>", escape_html(block));
        }
        Ok(out)
    }
}

/// The files of a publication, by path relative to its root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Site {
    /// File contents by relative path.
    pub files: BTreeMap<String, String>,
    /// Non-chapter files (images, attachments) the pages link to, relative
    /// to the book folder; the caller copies them next to the pages
    /// (under `src/` for an mdBook tree).
    pub assets: BTreeSet<String>,
}

/// Escapes text for HTML element content and quoted attributes.
#[must_use]
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// What a link target looks like in the published output.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavour {
    /// `.md` links, for mdBook (which turns them into `.html`).
    Markdown,
    /// `.html` links, `README.md` served as `index.html`.
    Html,
}

fn page_path(path: &ChapterPath, flavour: Flavour) -> String {
    match flavour {
        Flavour::Markdown => path.as_str().to_owned(),
        Flavour::Html => {
            if path.is_readme() {
                let dir = path.dir();
                if dir.is_empty() {
                    "index.html".to_owned()
                } else {
                    format!("{dir}/index.html")
                }
            } else {
                format!("{}.html", path.as_str().trim_end_matches(".md"))
            }
        }
    }
}

impl Book {
    /// The book as `audience` sees it: sections they may not see removed
    /// (without moving any other file, so page addresses stay stable).
    ///
    /// # Errors
    ///
    /// Returns an error only for an inconsistent book.
    pub fn published_view(&self, audience: &Audience) -> Result<Self> {
        let mut hidden = BTreeSet::new();
        for id in self.manifest.nodes.keys() {
            let access = self.access(id)?;
            let visible = match audience {
                Audience::Public => !access.is_restricted(),
                Audience::Members(principals) => access.permits(principals),
            };
            if !visible {
                let _ = hidden.insert(id.clone());
            }
        }
        let mut view = self.clone();
        let hidden_paths: BTreeSet<ChapterPath> = hidden
            .iter()
            .filter_map(|id| self.manifest.nodes.get(id).map(|e| e.path.clone()))
            .collect();
        for section in [Section::Prefix, Section::Numbered, Section::Suffix] {
            let items = view.summary.items_mut(section);
            items.retain(|item| match item {
                Item::Chapter(chapter) => !is_hidden(chapter, &hidden_paths),
                _ => true,
            });
            for item in items.iter_mut() {
                if let Item::Chapter(chapter) = item {
                    prune(chapter, &hidden_paths);
                }
            }
        }
        for id in &hidden {
            if let Some(entry) = view.manifest.nodes.remove(id) {
                let _ = view.files.remove(&entry.path);
            }
        }
        view.moved_away.clear();
        view.removed.clear();
        Ok(view)
    }

    /// The published Markdown of section `id` in this (already pruned)
    /// view: no front matter, a `#` title, labels filled in, links
    /// resolved for `flavour`. Asset links found are added to `assets`.
    fn published_page(
        &self,
        id: &NodeId,
        flavour: Flavour,
        assets: &mut BTreeSet<String>,
    ) -> Result<String> {
        let path = self.path_of(id)?.clone();
        let dir = path.dir().to_owned();
        let page_dir = page_path(&path, flavour)
            .rsplit_once('/')
            .map_or_else(String::new, |(d, _)| d.to_owned());
        let title = self.title_of(id)?.to_owned();
        let text = self.text(id)?;
        let body = markdown::body(text);
        let has_title = markdown::headings(body)
            .first()
            .is_some_and(|h| h.level == 1 && h.range.start == 0);
        let page = if has_title {
            body.to_owned()
        } else {
            format!("# {}\n\n{body}", markdown::escape_link_text(&title))
        };
        let chapter_paths: BTreeSet<&str> = self.files.keys().map(ChapterPath::as_str).collect();
        Ok(markdown::map_links(&page, |link, current| {
            if let Some((target, fragment)) = NodeId::from_uri(&link.dest) {
                let cross_ref = current.is_some_and(is_cross_ref_text);
                let Ok(target_path) = self.path_of(&target) else {
                    // Left out, or not in this book.
                    return Some(LinkEdit {
                        unlink: Some(if cross_ref {
                            "(unavailable section)".to_owned()
                        } else {
                            current.unwrap_or(&link.dest).to_owned()
                        }),
                        ..LinkEdit::default()
                    });
                };
                let mut href = path::relative(&page_dir, &page_path(target_path, flavour));
                if let Some(fragment) = fragment {
                    href.push('#');
                    href.push_str(fragment);
                }
                return Some(LinkEdit {
                    dest: Some(href),
                    text: if cross_ref {
                        self.label_of(&target).ok()
                    } else {
                        None
                    },
                    unlink: None,
                });
            }
            if link.kind == LinkKind::Autolink {
                return None;
            }
            let (target, fragment) = link
                .dest
                .split_once('#')
                .map_or((link.dest.as_str(), None), |(t, f)| (t, Some(f)));
            let resolved = path::resolve(&dir, target)?;
            if chapter_paths.contains(resolved.as_str()) {
                if flavour == Flavour::Markdown {
                    return None;
                }
                let target_path = ChapterPath::parse(&resolved).ok()?;
                let mut href = path::relative(&page_dir, &page_path(&target_path, flavour));
                if let Some(fragment) = fragment {
                    href.push('#');
                    href.push_str(fragment);
                }
                return Some(LinkEdit {
                    dest: Some(href),
                    ..LinkEdit::default()
                });
            }
            if std::path::Path::new(&resolved)
                .extension()
                .is_some_and(|e| e == "md")
            {
                // A link to a chapter that is left out or missing.
                return Some(LinkEdit {
                    unlink: Some(current.unwrap_or(&link.dest).to_owned()),
                    ..LinkEdit::default()
                });
            }
            let _ = assets.insert(resolved);
            None
        }))
    }

    /// The book as an mdBook source tree (`book.toml`, `src/SUMMARY.md`,
    /// `src/...`) for `audience`.
    ///
    /// # Errors
    ///
    /// Returns an error for an inconsistent book.
    pub fn to_mdbook(&self, audience: &Audience) -> Result<Site> {
        let started = Instant::now();
        let _span = tracing::info_span!("books.publish.mdbook", book.id = %self.id()).entered();
        let view = self.published_view(audience)?;
        let mut site = Site::default();
        let _ = site.files.insert(
            "book.toml".to_owned(),
            mdbook_toml(&view.manifest.title, "src", "book"),
        );
        let _ = site
            .files
            .insert("src/SUMMARY.md".to_owned(), view.summary_text());
        for (id, entry) in &view.manifest.nodes {
            let page = view.published_page(id, Flavour::Markdown, &mut site.assets)?;
            let _ = site.files.insert(format!("src/{}", entry.path), page);
        }
        record("mdbook", started, site.files.len());
        Ok(site)
    }

    /// The book as a self-contained static HTML site for `audience`,
    /// rendering each page with `renderer`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Render`] when a page fails to render.
    pub fn to_static_site(&self, audience: &Audience, renderer: &dyn Renderer) -> Result<Site> {
        let started = Instant::now();
        let _span = tracing::info_span!("books.publish.site", book.id = %self.id()).entered();
        let view = self.published_view(audience)?;
        let toc = view.toc();
        let order: Vec<&TocEntry> = toc.entries().into_iter().filter(|e| !e.draft).collect();
        let mut site = Site::default();
        for (index, entry) in order.iter().enumerate() {
            let (Some(id), Some(path)) = (&entry.id, &entry.path) else {
                continue;
            };
            let markdown = view.published_page(id, Flavour::Html, &mut site.assets)?;
            let html = renderer
                .render(&markdown)
                .map_err(|message| BookError::Render {
                    path: path.to_string(),
                    message,
                })?;
            let out_path = page_path(path, Flavour::Html);
            let previous = index.checked_sub(1).and_then(|i| order.get(i)).copied();
            let next = order.get(index + 1).copied();
            let page = page_html(&toc, &out_path, entry, &html, previous, next);
            let _ = site.files.insert(out_path, page);
        }
        if !site.files.contains_key("index.html") {
            let cover = cover_html(&toc, order.first().copied());
            let _ = site.files.insert("index.html".to_owned(), cover);
        }
        let _ = site.files.insert("style.css".to_owned(), STYLE.to_owned());
        let toc_json =
            serde_json::to_string_pretty(&toc).map_err(|e| BookError::Snapshot(e.to_string()))?;
        let _ = site.files.insert("toc.json".to_owned(), toc_json);
        record("site", started, site.files.len());
        Ok(site)
    }
}

fn record(kind: &'static str, started: Instant, files: usize) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        histogram.vellum.books.publish_ms = ms,
        publish.kind = kind,
        publish.files = files,
        "book published"
    );
}

fn is_hidden(chapter: &Chapter, hidden: &BTreeSet<ChapterPath>) -> bool {
    chapter
        .location
        .as_ref()
        .is_some_and(|p| hidden.contains(p))
}

fn prune(chapter: &mut Chapter, hidden: &BTreeSet<ChapterPath>) {
    chapter.children.retain(|c| !is_hidden(c, hidden));
    for child in &mut chapter.children {
        prune(child, hidden);
    }
}

/// A `book.toml` for mdBook.
#[must_use]
pub fn mdbook_toml(title: &str, src: &str, build_dir: &str) -> String {
    format!(
        "# Generated by Vellum from book.yaml; edits are overwritten.\n\
         [book]\ntitle = {}\nsrc = {}\n\n[build]\nbuild-dir = {}\ncreate-missing = false\n",
        toml_string(title),
        toml_string(src),
        toml_string(build_dir)
    )
}

fn toml_string(value: &str) -> String {
    let mut out = String::from('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn nav_html(toc: &Toc, current: &str) -> String {
    let root = root_prefix(current);
    let mut out = String::from("<ol class=\"toc\">\n");
    for item in &toc.items {
        match item {
            TocItem::Chapter(entry) => nav_entry(&mut out, entry, current, &root),
            TocItem::Part { title } => {
                let _ = writeln!(out, "<li class=\"part\">{}</li>", escape_html(title));
            }
            TocItem::Separator => out.push_str("<li class=\"sep\" role=\"separator\"></li>\n"),
        }
    }
    out.push_str("</ol>\n");
    out
}

fn nav_entry(out: &mut String, entry: &TocEntry, current: &str, root: &str) {
    out.push_str("<li>");
    let number = entry
        .number_label
        .as_ref()
        .map(|n| format!("<span class=\"num\">{}</span> ", escape_html(n)))
        .unwrap_or_default();
    match &entry.path {
        Some(path) => {
            let target = page_path(path, Flavour::Html);
            let aria = if target == current {
                " aria-current=\"page\""
            } else {
                ""
            };
            let _ = write!(
                out,
                "<a href=\"{root}{}\"{aria}>{number}{}</a>",
                escape_html(&target),
                escape_html(&entry.title)
            );
        }
        None => {
            let _ = write!(
                out,
                "<span class=\"draft\">{number}{}</span>",
                escape_html(&entry.title)
            );
        }
    }
    if !entry.children.is_empty() {
        out.push_str("\n<ol>\n");
        for child in &entry.children {
            nav_entry(out, child, current, root);
        }
        out.push_str("</ol>\n");
    }
    out.push_str("</li>\n");
}

fn root_prefix(page: &str) -> String {
    "../".repeat(page.matches('/').count())
}

fn shell(toc: &Toc, current: &str, title: &str, main: &str) -> String {
    let root = root_prefix(current);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"generator\" content=\"Vellum\">\n\
         <title>{title}</title>\n<link rel=\"stylesheet\" href=\"{root}style.css\">\n</head>\n<body>\n\
         <nav class=\"sidebar\" aria-label=\"Contents\">\n<a class=\"book\" href=\"{root}index.html\">{book}</a>\n{nav}</nav>\n\
         <main>\n{main}</main>\n</body>\n</html>\n",
        title = escape_html(title),
        book = escape_html(&toc.title),
        nav = nav_html(toc, current),
    )
}

fn page_html(
    toc: &Toc,
    current: &str,
    entry: &TocEntry,
    html: &str,
    previous: Option<&TocEntry>,
    next: Option<&TocEntry>,
) -> String {
    let root = root_prefix(current);
    let link = |e: &TocEntry, rel: &str, class: &str| {
        e.path.as_ref().map_or_else(String::new, |p| {
            format!(
                "<a class=\"{class}\" rel=\"{rel}\" href=\"{root}{}\">{}</a>",
                escape_html(&page_path(p, Flavour::Html)),
                escape_html(&e.label)
            )
        })
    };
    let pager = format!(
        "<nav class=\"pager\" aria-label=\"Previous and next\">{}{}</nav>\n",
        previous
            .map(|e| link(e, "prev", "prev"))
            .unwrap_or_default(),
        next.map(|e| link(e, "next", "next")).unwrap_or_default(),
    );
    let main = format!("<article>\n{html}</article>\n{pager}");
    shell(
        toc,
        current,
        &format!("{} · {}", entry.label, toc.title),
        &main,
    )
}

fn cover_html(toc: &Toc, first: Option<&TocEntry>) -> String {
    let start = first
        .and_then(|e| e.path.as_ref())
        .map(|p| {
            format!(
                "<p><a class=\"start\" href=\"{}\">Start reading</a></p>\n",
                escape_html(&page_path(p, Flavour::Html))
            )
        })
        .unwrap_or_default();
    let main = format!(
        "<article>\n<h1>{}</h1>\n{start}</article>\n",
        escape_html(&toc.title)
    );
    shell(toc, "index.html", &toc.title, &main)
}

const STYLE: &str = r#"/* Generated by Vellum. Material 3 tokens from the Vellum design. */
:root {
  --primary: #4f5b92; --primary-container: #dde1ff;
  --surface: #f7f8fc; --surface-2: #f0f2f9; --surface-3: #e6e8ee;
  --outline: #c5c6d0; --outline-strong: #767680;
  --text: #1d1b20; --text-muted: #46464f;
  color-scheme: light dark;
}
@media (prefers-color-scheme: dark) {
  :root {
    --primary: #b8c3ff; --primary-container: #37447a;
    --surface: #121318; --surface-2: #1a1b21; --surface-3: #292a2f;
    --outline: #46464f; --outline-strong: #90909a;
    --text: #e3e1e9; --text-muted: #c6c5d0;
  }
}
* { box-sizing: border-box; }
body { margin: 0; display: grid; grid-template-columns: minmax(14rem, 20rem) 1fr;
  font: 16px/1.6 Roboto, system-ui, sans-serif; background: var(--surface); color: var(--text); }
.sidebar { background: var(--surface-2); border-right: 1px solid var(--outline);
  padding: 1rem; height: 100vh; position: sticky; top: 0; overflow-y: auto; }
.sidebar .book { display: block; font-weight: 600; margin-bottom: 1rem; color: var(--text); text-decoration: none; }
.toc, .toc ol { list-style: none; margin: 0; padding-left: 1rem; }
.toc { padding-left: 0; }
.toc a { color: var(--text); text-decoration: none; border-radius: 1rem; padding: 0.125rem 0.5rem; display: inline-block; }
.toc a:hover, .toc a:focus-visible { background: var(--surface-3); }
.toc a[aria-current="page"] { background: var(--primary-container); }
.toc .num { color: var(--text-muted); }
.toc .part { margin-top: 1rem; font-weight: 600; color: var(--text-muted); }
.toc .sep { border-top: 1px solid var(--outline); margin: 0.5rem 0; }
.toc .draft { color: var(--text-muted); font-style: italic; padding: 0.125rem 0.5rem; }
main { padding: 2rem clamp(1rem, 5vw, 4rem); max-width: 52rem; }
a { color: var(--primary); }
.pager { display: flex; justify-content: space-between; gap: 1rem; margin-top: 3rem;
  border-top: 1px solid var(--outline); padding-top: 1rem; }
.pager .next { margin-left: auto; }
@media (max-width: 48rem) {
  body { grid-template-columns: 1fr; }
  .sidebar { position: static; height: auto; border-right: 0; border-bottom: 1px solid var(--outline); }
}
@media (prefers-reduced-motion: reduce) { * { transition: none !important; } }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};

    fn restricted() -> Book {
        let mut book = handbook();
        let staff = BTreeSet::from([Principal::parse("group:staff").unwrap()]);
        book.set_restriction(&id(4), Some(staff)).unwrap();
        book
    }

    #[test]
    fn public_view_leaves_out_restricted_sections() {
        let book = restricted();
        let view = book.published_view(&Audience::Public).unwrap();
        assert!(!view.contains(&id(4)) && !view.contains(&id(5)));
        assert!(!view.summary_text().contains("Architecture"));
        // Other files keep their paths.
        assert_eq!(view.path_of(&id(3)).unwrap(), book.path_of(&id(3)).unwrap());
        let members = Audience::Members(BTreeSet::from([Principal::parse("group:staff").unwrap()]));
        assert!(book.published_view(&members).unwrap().contains(&id(5)));
    }

    #[test]
    fn mdbook_tree() {
        let site = restricted().to_mdbook(&Audience::Public).unwrap();
        assert!(site.files.contains_key("book.toml"));
        let week = site.files.get("src/onboarding/first-week.md").unwrap();
        assert!(week.starts_with("# Your first week\n\n"), "{week}");
        assert!(!week.contains("id:"), "{week}");
        // Services is restricted: the cross-reference becomes plain text.
        assert!(week.contains("Read (unavailable section) first."), "{week}");
        let site = handbook().to_mdbook(&Audience::Public).unwrap();
        let week = site.files.get("src/onboarding/first-week.md").unwrap();
        assert!(
            week.contains("[§2.1 Services](../architecture/services.md)"),
            "{week}"
        );
        let services = site.files.get("src/architecture/services.md").unwrap();
        assert!(services.contains("[the glossary](../glossary.md)"));
        assert!(
            services.contains("[§1 Onboarding](../onboarding/README.md)"),
            "{services}"
        );
    }

    #[test]
    fn static_site() {
        let site = handbook()
            .to_static_site(&Audience::Public, &PlainRenderer)
            .unwrap();
        let services = site.files.get("architecture/services.html").unwrap();
        assert!(services.contains("href=\"../style.css\""));
        assert!(services.contains("<title>§2.1 Services · Northstar handbook</title>"));
        assert!(services.contains("aria-current=\"page\""));
        assert!(services.contains("rel=\"prev\""));
        // Links between pages point at .html files; README becomes index.html.
        assert!(services.contains("../glossary.html"), "{services}");
        assert!(services.contains("../onboarding/index.html"), "{services}");
        assert!(site.files.contains_key("index.html"));
        assert!(site.files.contains_key("onboarding/index.html"));
        assert!(site.files.contains_key("toc.json"));
        // The draft is listed but not linked.
        assert!(services.contains(
            "<span class=\"draft\"><span class=\"num\">1.2</span> Security training</span>"
        ));
        assert!(!services.contains("<script"));
    }

    #[test]
    fn escaping() {
        assert_eq!(
            escape_html("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        let plain = PlainRenderer.render("<script>alert(1)</script>").unwrap();
        assert!(!plain.contains("<script>"));
        assert_eq!(
            mdbook_toml("A \"quoted\" title", "src", "book"),
            "# Generated by Vellum from book.yaml; edits are overwritten.\n[book]\ntitle = \"A \\\"quoted\\\" title\"\nsrc = \"src\"\n\n[build]\nbuild-dir = \"book\"\ncreate-missing = false\n"
        );
    }
}
