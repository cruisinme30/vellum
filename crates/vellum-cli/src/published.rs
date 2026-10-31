// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Published books (ADR 0013, D237).
//!
//! - [`SiteRenderer`] renders each page of a published site with the
//!   render service, offline (diagrams come from the cache), so a site
//!   looks like the book does in the app.
//! - [`router`] serves a book's public site at
//!   `/published/books/{book_id}/{path}`. Only a site published for
//!   everyone is served; anything else is "not found", whoever asks. The
//!   pages carry the public origin's policy (no scripts, sandboxed).

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use http::{HeaderValue, StatusCode, header};
use vellum_books_service::BooksService;
use vellum_kernel::BookId;
use vellum_render::figure::Figures;
use vellum_render::resolve::NoResolver;
use vellum_render::{RenderOptions, Renderer};

/// Renders book pages for a published site.
#[derive(Debug, Clone)]
pub struct SiteRenderer(Renderer);

impl SiteRenderer {
    /// A renderer drawing figures with `figures`.
    #[must_use]
    pub const fn new(figures: Figures) -> Self {
        Self(Renderer::new(figures))
    }
}

impl vellum_books::publish::Renderer for SiteRenderer {
    fn render(&self, markdown: &str) -> Result<String, String> {
        let doc = vellum_format::parse(markdown).document;
        Ok(self
            .0
            .render_offline(&doc, &NoResolver, &RenderOptions::default())
            .html)
    }
}

/// The public-site routes, to mount at `/published`.
pub fn router(books: Arc<BooksService>) -> Router {
    Router::new()
        .route("/books/{book_id}", get(root))
        .route("/books/{book_id}/", get(root))
        .route("/books/{book_id}/{*path}", get(file))
        .with_state(books)
}

async fn root(
    State(books): State<Arc<BooksService>>,
    Path(book): Path<String>,
) -> Response {
    serve(&books, &book, "").await
}

async fn file(
    State(books): State<Arc<BooksService>>,
    Path((book, path)): Path<(String, String)>,
) -> Response {
    serve(&books, &book, &path).await
}

async fn serve(books: &BooksService, book: &str, path: &str) -> Response {
    let Ok(id) = book.parse::<BookId>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match books.public_file(id, path).await {
        Ok(Some(bytes)) => {
            let name = if path.is_empty() || path.ends_with('/') {
                "index.html"
            } else {
                path
            };
            let mut response = Response::new(Body::from(bytes));
            let headers = response.headers_mut();
            let _ = headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(crate::web::content_type(name)),
            );
            let _ = headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            );
            let _ = headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(vellum_api::security::public_csp()),
            );
            response
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "published site file could not be read");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}
