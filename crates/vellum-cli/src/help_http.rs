// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The help books in the browser, mounted at `/guide` (ADR 0024, D256).
//!
//! The app's `?` buttons link to `/guide/topics/<help ID>`, which redirects
//! to the page and heading `docs/help-anchors.yaml` names. Pages are the
//! books built into this binary ([`vellum_help`]), rendered by
//! [`vellum_render`] exactly as Vellum renders any page, so the help works
//! offline, matches the server's version and shows every block the way the
//! reader's own pages show it.
//!
//! - `/guide/` lists the books; `/guide/<book>/` is a book's introduction.
//! - `/guide/<book>/<path>.html` is the page `<path>.md`; relative `.md`
//!   links in it become `.html`, so links between chapters keep working.
//! - `/guide/<book>/<asset>` serves an embedded image.
//! - `/guide/vellum.css` is the theme stylesheet plus the reader's layout.
//!
//! Every path is looked up in the embedded tables, never on disk. Pages
//! carry no script, and rendered HTML is the renderer's sanitised output.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use axum::Router;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use http::{HeaderValue, StatusCode, header};
use vellum_help::{Book, Page};
use vellum_render::figure::Figures;
use vellum_render::resolve::NoResolver;
use vellum_render::{RenderOptions, Renderer};

/// Where the books are mounted.
pub const PREFIX: &str = "/guide";

/// Where a page's source lives, for "Suggest an edit".
const EDIT_BASE: &str =
    "https://github.com/cruisinme30/vellum/edit/develop/docs";

/// The reader's own layout, after the theme stylesheet.
const READER_CSS: &str = include_str!("help_reader.css");

static STYLESHEET: LazyLock<String> = LazyLock::new(|| {
    let mut css = vellum_render::theme::stylesheet();
    css.push_str(READER_CSS);
    css
});

struct Reader {
    renderer: Renderer,
    /// Rendered pages by `book/path`. Only pages whose diagrams all drew
    /// are kept, so a diagram server that was briefly down is retried.
    cache: Mutex<HashMap<String, Arc<str>>>,
}

/// The `/guide` router, drawing diagrams with `figures`.
pub fn router(figures: Figures) -> Router {
    let reader = Arc::new(Reader {
        renderer: Renderer::new(figures),
        cache: Mutex::new(HashMap::new()),
    });
    Router::new()
        .route("/", get(index))
        .route("/vellum.css", get(stylesheet))
        .route("/topics/{id}", get(topic))
        .route("/{book}", get(book_root))
        .route("/{book}/", get(book_home))
        .route("/{book}/{*path}", get(book_file))
        .with_state(reader)
}

/// The URL of a help ID's page and heading, if the ID exists.
#[must_use]
pub fn topic_url(help_id: &str) -> Option<String> {
    let target = vellum_help::resolve(help_id)?;
    let mut url = page_url(target.page);
    if let Some(anchor) = target.anchor {
        // The renderer prefixes every element id it writes.
        url.push('#');
        url.push_str(vellum_render::ID_PREFIX);
        url.push_str(anchor);
    }
    Some(url)
}

fn page_url(page: &Page) -> String {
    format!("{PREFIX}/{}/{}", page.book().slug(), html_name(page.path()))
}

/// `writing/text.md` → `writing/text.html`.
fn html_name(path: &str) -> String {
    path.strip_suffix(".md")
        .map_or_else(|| path.to_owned(), |stem| format!("{stem}.html"))
}

async fn index() -> Response {
    let mut body =
        String::from("<h1>Vellum help</h1>\n<ul class=\"vl-help-books\">\n");
    for book in Book::ALL {
        let _ = writeln!(
            body,
            "<li><a href=\"{PREFIX}/{}/\">{}</a> <span>{} pages</span></li>",
            book.slug(),
            escape(book.title()),
            book.pages().count()
        );
    }
    body.push_str("</ul>\n");
    html(StatusCode::OK, &document("Vellum help", None, &body, ""))
}

async fn stylesheet() -> Response {
    let mut response = STYLESHEET.as_str().into_response();
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    let _ = headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

async fn topic(Path(id): Path<String>) -> Response {
    topic_url(&id)
        .map_or_else(not_found, |url| Redirect::to(&url).into_response())
}

async fn book_root(Path(book): Path<String>) -> Response {
    Book::from_slug(&book).map_or_else(not_found, |book| {
        Redirect::permanent(&format!("{PREFIX}/{}/", book.slug()))
            .into_response()
    })
}

async fn book_home(
    State(reader): State<Arc<Reader>>,
    Path(book): Path<String>,
) -> Response {
    let Some(book) = Book::from_slug(&book) else {
        return not_found();
    };
    match vellum_help::page(book, "README.md") {
        Some(page) => reader.page(page).await,
        None => not_found(),
    }
}

async fn book_file(
    State(reader): State<Arc<Reader>>,
    Path((book, path)): Path<(String, String)>,
) -> Response {
    let Some(book) = Book::from_slug(&book) else {
        return not_found();
    };
    if let Some(stem) = path.strip_suffix(".html") {
        return match vellum_help::page(book, &format!("{stem}.md")) {
            Some(page) => reader.page(page).await,
            None => not_found(),
        };
    }
    let Some(asset) = vellum_help::asset(book, &path) else {
        return not_found();
    };
    let mut response = asset.bytes().into_response();
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset_type(&path)),
    );
    let _ = headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

impl Reader {
    async fn page(&self, page: &'static Page) -> Response {
        let key = format!("{}/{}", page.book().slug(), page.path());
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(body) = cached {
            return html(StatusCode::OK, &body);
        }
        let doc = vellum_format::parse(page.markdown()).document;
        let options = RenderOptions {
            doc_key: Some(format!("help:{key}")),
        };
        let rendered = self.renderer.render(&doc, &NoResolver, &options).await;
        if !rendered.warnings.is_empty() {
            tracing::warn!(
                page = %key,
                warnings = rendered.warnings.len(),
                "help page rendered with fallbacks"
            );
        }
        let body: Arc<str> = Arc::from(document(
            page.title(),
            Some(page),
            &md_links_to_html(&rendered.html),
            &contents(page.book(), page.path()),
        ));
        if rendered.stale_figures == 0 {
            let _ = self
                .cache
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, Arc::clone(&body));
        }
        html(StatusCode::OK, &body)
    }
}

/// A whole help page: the site bar, the book's contents, the page and its
/// footer.
fn document(
    title: &str,
    page: Option<&Page>,
    main: &str,
    contents: &str,
) -> String {
    let book_title = page.map(|p| p.book().title());
    let full_title = book_title.map_or_else(
        || escape(title),
        |book| format!("{} · {}", escape(title), escape(book)),
    );
    let mut nav =
        format!("<a class=\"vl-help-home\" href=\"{PREFIX}/\">Vellum help</a>");
    for book in Book::ALL {
        let current = page.is_some_and(|p| p.book() == book);
        let _ = write!(
            nav,
            "<a href=\"{PREFIX}/{}/\"{}>{}</a>",
            book.slug(),
            if current {
                " aria-current=\"true\""
            } else {
                ""
            },
            escape(book.title())
        );
    }
    let footer = page.map_or_else(String::new, |p| {
        format!(
            "<footer class=\"vl-help-footer\"><a \
             href=\"{EDIT_BASE}/{}.book/{}\" rel=\"noopener\">Suggest an \
             edit</a> <span>Help for this server's version of \
             Vellum.</span></footer>",
            p.book().slug(),
            escape(p.path())
        )
    });
    let aside = if contents.is_empty() {
        String::new()
    } else {
        format!(
            "<nav class=\"vl-help-contents\" \
                aria-label=\"Contents\">{contents}</nav>"
        )
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, \
         initial-scale=1\">\n<title>{full_title}</title>\n\
         <link rel=\"stylesheet\" href=\"{PREFIX}/vellum.css\">\n</head>\n\
         <body class=\"vl-help\">\n\
         <header class=\"vl-help-bar\"><nav aria-label=\"Help \
         books\">{nav}</nav></header>\n\
         <div class=\"vl-help-layout\">{aside}<main id=\"main\">\n{main}\n\
         {footer}</main></div>\n</body>\n</html>\n"
    )
}

/// The book's table of contents from its `SUMMARY.md`, with the current
/// page marked.
fn contents(book: Book, current: &str) -> String {
    let mut out = String::from("<ul>");
    for line in book.summary().lines() {
        let trimmed = line.trim_start();
        if let Some(part) = trimmed.strip_prefix("# ") {
            if part != "Summary" {
                let _ = write!(
                    out,
                    "<li class=\"vl-help-part\">{}</li>",
                    escape(part)
                );
            }
            continue;
        }
        let entry = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        let Some((title, target)) = summary_link(entry) else {
            continue;
        };
        let nested = line.len() - trimmed.len() >= 2;
        let marker = if target == current {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(
            out,
            "<li{}><a href=\"{PREFIX}/{}/{}\"{marker}>{}</a></li>",
            if nested {
                " class=\"vl-help-nested\""
            } else {
                ""
            },
            book.slug(),
            escape(&html_name(target)),
            escape(title)
        );
    }
    out.push_str("</ul>");
    out
}

/// `[Title](path.md)` → `("Title", "path.md")`.
fn summary_link(entry: &str) -> Option<(&str, &str)> {
    let rest = entry.strip_prefix('[')?;
    let (title, rest) = rest.split_once("](")?;
    let (target, _) = rest.split_once(')')?;
    Some((title, target))
}

/// Rewrites relative `href="….md"` and `href="….md#x"` links to `.html`,
/// and `<name>.book/` to `<name>/` (a link to another book), leaving
/// absolute URLs and everything else alone.
fn md_links_to_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("href=\"") {
        let (before, after) = rest.split_at(start + 6);
        out.push_str(before);
        let end = after.find('"').unwrap_or(after.len());
        let (target, tail) = after.split_at(end);
        let (path, fragment) = target
            .split_once('#')
            .map_or((target, None), |(p, f)| (p, Some(f)));
        let relative =
            !path.contains(':') && !path.starts_with('/') && !path.is_empty();
        match path.strip_suffix(".md") {
            Some(stem) if relative => {
                out.push_str(&stem.replace(".book/", "/"));
                out.push_str(".html");
                if let Some(fragment) = fragment {
                    out.push('#');
                    out.push_str(fragment);
                }
            }
            _ => out.push_str(target),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn asset_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

fn escape(text: &str) -> String {
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

fn html(status: StatusCode, body: &str) -> Response {
    let mut response = (status, body.to_owned()).into_response();
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let _ = headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn not_found() -> Response {
    let body = format!(
        "<h1>No such help page</h1>\n\
         <p><a href=\"{PREFIX}/\">See every help book</a>, or run <code>vellum \
         docs</code> on the server.</p>\n"
    );
    html(
        StatusCode::NOT_FOUND,
        &document("Help page not found", None, &body, ""),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use http::Request;
    use tower::ServiceExt as _;
    use vellum_render::figure::FigureConfig;

    use super::*;

    async fn get(path: &str) -> (StatusCode, Option<String>, String) {
        let app = router(Figures::new(FigureConfig::default()).unwrap());
        let response = app
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            location,
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    #[tokio::test]
    async fn every_help_id_redirects_to_a_page_that_renders() {
        for id in vellum_help::help_ids() {
            let (status, location, _) = get(&format!("/topics/{id}")).await;
            assert_eq!(status, StatusCode::SEE_OTHER, "{id}");
            let location = location.unwrap();
            assert!(location.starts_with("/guide/"), "{id}: {location}");
            let path = location
                .strip_prefix(PREFIX)
                .unwrap()
                .split('#')
                .next()
                .unwrap();
            let (status, _, body) = get(path).await;
            assert_eq!(status, StatusCode::OK, "{id}: {path}");
            if let Some((_, anchor)) = location.split_once('#') {
                assert!(
                    body.contains(&format!("id=\"{anchor}\"")),
                    "{id}: #{anchor}"
                );
                assert!(anchor.starts_with(vellum_render::ID_PREFIX), "{id}");
            }
        }
        let (status, _, _) = get("/topics/no.such.topic").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn pages_link_to_html_and_carry_no_script() {
        let (status, _, body) = get("/user/writing/text.html").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(
                "<link rel=\"stylesheet\" href=\"/guide/vellum.css\">"
            )
        );
        assert!(
            body.contains("aria-current=\"page\""),
            "contents mark the page"
        );
        let md_link = body.split("href=\"").skip(1).find(|rest| {
            let target = rest.split('"').next().unwrap_or_default();
            !target.contains(':')
                && target
                    .split('#')
                    .next()
                    .unwrap_or_default()
                    .rsplit_once('.')
                    .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
        });
        assert_eq!(md_link, None, "every chapter link is .html");
        assert!(!body.to_ascii_lowercase().contains("<script"));
        assert!(body.contains("/edit/develop/docs/user.book/writing/text.md"));

        let (status, _, home) = get("/user/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(home.contains("<title>"));
        let (status, location, _) = get("/admin").await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        assert_eq!(location.as_deref(), Some("/guide/admin/"));
        let (status, _, index) = get("/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(index.contains("/guide/developer/"));
    }

    #[tokio::test]
    async fn assets_and_the_stylesheet_are_served_and_nothing_else() {
        let (status, _, svg) =
            get("/user/writing/assets/request-path.svg").await;
        assert_eq!(status, StatusCode::OK);
        assert!(svg.starts_with("<?xml"));
        let (status, _, css) = get("/vellum.css").await;
        assert_eq!(status, StatusCode::OK);
        assert!(css.contains(".vl-help-layout"));
        for path in [
            "/user/../../Cargo.toml",
            "/user/%2e%2e/admin/README.html",
            "/user/no-such-page.html",
            "/nobody/README.html",
            "/user/README.md",
        ] {
            let (status, _, _) = get(path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn only_relative_markdown_links_are_rewritten() {
        let html = concat!(
            r#"<a href="../data/views.md#filters">a</a><a "#,
            r#"href="https://x.dev/a.md">b</a><a href="/abs.md">c</a><a "#,
            r#"href="README.md">d</a><a href="other.txt">e</a><a "#,
            r#"href="../../admin.book/deploy/README.md">f</a>"#,
        );
        assert_eq!(
            md_links_to_html(html),
            concat!(
                r#"<a href="../data/views.html#filters">a</a><a "#,
                r#"href="https://x.dev/a.md">b</a><a href="/abs.md">c</a><a "#,
                r#"href="README.html">d</a><a href="other.txt">e</a><a "#,
                r#"href="../../admin/deploy/README.html">f</a>"#,
            )
        );
        assert_eq!(
            escape("<a href=\"x\">'"),
            "&lt;a href=&quot;x&quot;&gt;&#39;"
        );
    }
}
