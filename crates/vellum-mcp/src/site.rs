// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Markdown for any model or crawler: `/llms.txt`, `/sitemap.md`, and
//! `?format=md` on every page URL.
//!
//! These are plain HTTP, not MCP. They show what the visitor may read: an
//! anonymous visitor sees public pages only. The application puts the
//! signed-in [`Actor`] in the request extensions when there is one.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::ports::{Actor, Listing, Workspace, WorkspaceError};
use crate::safety;

/// Most entries listed in `llms.txt` and the sitemap.
const MAX_ENTRIES: usize = 5_000;

async fn listings(
    ws: &dyn Workspace,
    actor: &Actor,
) -> Result<Vec<Listing>, WorkspaceError> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = ws.list_readable(actor, cursor, 500).await?;
        out.extend(page.items);
        match page.next_cursor {
            Some(c) if out.len() < MAX_ENTRIES => cursor = Some(c),
            _ => break,
        }
    }
    out.truncate(MAX_ENTRIES);
    Ok(out)
}

fn md_url(url: &str) -> String {
    if url.contains('?') {
        format!("{url}&format=md")
    } else {
        format!("{url}?format=md")
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['[', ']'], "")
}

fn grouped(items: &[Listing]) -> BTreeMap<&str, Vec<&Listing>> {
    let mut by_space: BTreeMap<&str, Vec<&Listing>> = BTreeMap::new();
    for l in items {
        by_space
            .entry(if l.space.is_empty() {
                "General"
            } else {
                &l.space
            })
            .or_default()
            .push(l);
    }
    by_space
}

fn entry(l: &Listing) -> String {
    let desc = l
        .description
        .as_deref()
        .map_or(String::new(), |d| format!(": {}", one_line(d)));
    format!("- [{}]({}){desc}\n", one_line(&l.title), md_url(&l.url))
}

/// Builds `llms.txt` (the llmstxt.org format) for an actor.
///
/// # Errors
///
/// When the workspace cannot be listed.
pub async fn llms_txt(
    ws: &dyn Workspace,
    actor: &Actor,
) -> Result<String, WorkspaceError> {
    let site = ws.site_info().await?;
    let items = listings(ws, actor).await?;
    let base = site.base_url.trim_end_matches('/');
    let mut out = format!(
        "# {}\n\n> {}\n\n\
            This is a Vellum workspace. Every page is available as clean \
            Markdown: add `?format=md` to its URL. Models can connect over MCP \
            at {base}/mcp.\n\n",
        one_line(&site.name),
        one_line(&site.summary)
    );
    for (space, entries) in grouped(&items) {
        let _ = writeln!(out, "## {}\n", one_line(space));
        for l in entries {
            out.push_str(&entry(l));
        }
        out.push('\n');
    }
    let _ = write!(
        out,
        "## Optional\n\n\
            - [Markdown sitemap]({base}/sitemap.md): every page, grouped by \
            space\n"
    );
    Ok(out)
}

/// Builds the Markdown sitemap for an actor.
///
/// # Errors
///
/// When the workspace cannot be listed.
pub async fn sitemap(
    ws: &dyn Workspace,
    actor: &Actor,
) -> Result<String, WorkspaceError> {
    let site = ws.site_info().await?;
    let items = listings(ws, actor).await?;
    let mut out = format!(
        "# {} sitemap\n\n{} pages. Each link returns Markdown.\n\n",
        one_line(&site.name),
        items.len()
    );
    for (space, entries) in grouped(&items) {
        let _ = writeln!(out, "## {}\n", one_line(space));
        for l in entries {
            out.push_str(&entry(l));
        }
        out.push('\n');
    }
    Ok(out)
}

fn actor_of(req: &Request) -> Actor {
    req.extensions()
        .get::<Actor>()
        .cloned()
        .unwrap_or(Actor::Anonymous)
}

fn markdown(
    body: String,
    content_type: &'static str,
    public: bool,
) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    let _ =
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    let _ = h.insert(
        header::VARY,
        HeaderValue::from_static("Cookie, Authorization"),
    );
    let _ = h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if public {
            "public, max-age=300"
        } else {
            "private, max-age=60"
        }),
    );
    r
}

fn failure(e: &WorkspaceError) -> Response {
    match e {
        WorkspaceError::NotFound(_) | WorkspaceError::Forbidden(_) => {
            StatusCode::NOT_FOUND.into_response()
        }
        WorkspaceError::Unavailable(_) => {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn llms_handler(
    State(ws): State<Arc<dyn Workspace>>,
    req: Request,
) -> Response {
    let actor = actor_of(&req);
    let public = actor == Actor::Anonymous;
    match llms_txt(ws.as_ref(), &actor).await {
        Ok(text) => markdown(text, "text/plain; charset=utf-8", public),
        Err(e) => failure(&e),
    }
}

async fn sitemap_handler(
    State(ws): State<Arc<dyn Workspace>>,
    req: Request,
) -> Response {
    let actor = actor_of(&req);
    let public = actor == Actor::Anonymous;
    match sitemap(ws.as_ref(), &actor).await {
        Ok(text) => markdown(text, "text/markdown; charset=utf-8", public),
        Err(e) => failure(&e),
    }
}

/// Routes for `/llms.txt` and `/sitemap.md`.
pub fn router(ws: Arc<dyn Workspace>) -> Router {
    Router::new()
        .route("/llms.txt", get(llms_handler))
        .route("/sitemap.md", get(sitemap_handler))
        .with_state(ws)
}

/// `llms.txt` at the root of its router, for a gateway that nests it at
/// `/llms.txt`.
pub fn llms_router(ws: Arc<dyn Workspace>) -> Router {
    Router::new().route("/", get(llms_handler)).with_state(ws)
}

/// The Markdown sitemap at the root of its router, for a gateway that
/// nests it at `/sitemap.md`.
pub fn sitemap_router(ws: Arc<dyn Workspace>) -> Router {
    Router::new()
        .route("/", get(sitemap_handler))
        .with_state(ws)
}

fn wants_markdown(req: &Request) -> bool {
    req.method() == Method::GET
        && req
            .uri()
            .query()
            .is_some_and(|q| q.split('&').any(|p| p == "format=md"))
}

/// Middleware for the page routes: `?format=md` returns Markdown.
///
/// A GET with `?format=md` gets the page as Vellum Markdown (front matter
/// included, invisible text stripped) instead of HTML. Mount it with
/// `axum::middleware::from_fn_with_state(workspace, site::format_md)`.
pub async fn format_md(
    State(ws): State<Arc<dyn Workspace>>,
    req: Request,
    next: Next,
) -> Response {
    if !wants_markdown(&req) {
        return next.run(req).await;
    }
    let actor = actor_of(&req);
    let public = actor == Actor::Anonymous;
    let path = req.uri().path().to_owned();
    match ws.get_doc_by_path(&actor, &path).await {
        Ok(doc) => markdown(
            safety::guard(&doc.markdown).text,
            "text/markdown; charset=utf-8",
            public,
        ),
        Err(WorkspaceError::NotFound(_)) => next.run(req).await,
        Err(e) => failure(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_urls_keep_existing_queries() {
        assert_eq!(md_url("https://x/a"), "https://x/a?format=md");
        assert_eq!(md_url("https://x/a?v=1"), "https://x/a?v=1&format=md");
    }

    #[test]
    fn titles_cannot_break_links() {
        assert_eq!(one_line("A [tricky]\n title"), "A tricky title");
    }
}
