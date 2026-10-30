// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The public-pages origin: share links and published pages.
//!
//! Public content is served from its own origin (`pages.northstar.dev`, not
//! the app's), with no cookies from the app, no scripts, and a sandboxing
//! CSP, so user content can never act with a signed-in user's authority.
//!
//! - `GET /s/{token}` opens a share link. Password-protected links answer
//!   with a password form; `POST /s/{token}` submits it.
//! - `GET /p/{*path}` serves a page published to the public site.
//!
//! The gateway owns the routes and headers; resolving tokens and rendering
//! pages is behind the [`PublicSite`] port, which the binary wires to the
//! identity (share records) and pages (rendered HTML) services.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::{HeaderValue, StatusCode, header};
use serde::Deserialize;
use vellum_contracts::BoxFuture;
use vellum_kernel::{Error, ErrorKind, ShareToken};

/// Rendered public content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicPage {
    /// A rendered, sanitised HTML document.
    Html {
        /// The document.
        html: String,
        /// Seconds shared caches may keep it; `0` for private content.
        max_age: u32,
    },
    /// The link needs a password; show the form (again, if `retry`).
    PasswordRequired {
        /// Whether a wrong password was just given.
        retry: bool,
    },
}

/// Resolves share links and published pages to rendered HTML.
pub trait PublicSite: Send + Sync + 'static {
    /// Opens a share link, checking expiry, revocation and the password.
    /// Unknown, expired and revoked links are all `not_found`.
    fn shared(
        &self,
        token: ShareToken,
        password: Option<String>,
    ) -> BoxFuture<'_, Result<PublicPage, Error>>;

    /// Serves a page published to the public site.
    fn published<'a>(
        &'a self,
        path: &'a str,
    ) -> BoxFuture<'a, Result<PublicPage, Error>>;
}

/// The public site when nothing is published: every path is not found.
#[derive(Debug, Clone, Copy, Default)]
pub struct NothingPublished;

impl PublicSite for NothingPublished {
    fn shared(
        &self,
        _token: ShareToken,
        _password: Option<String>,
    ) -> BoxFuture<'_, Result<PublicPage, Error>> {
        Box::pin(async {
            Err(Error::new(ErrorKind::NotFound, "share link not found"))
        })
    }

    fn published<'a>(
        &'a self,
        _path: &'a str,
    ) -> BoxFuture<'a, Result<PublicPage, Error>> {
        Box::pin(async {
            Err(Error::new(ErrorKind::NotFound, "page not found"))
        })
    }
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            c => c.to_string(),
        })
        .collect()
}

fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta \
         name=\"viewport\" content=\"width=device-width, \
         initial-scale=1\"><meta name=\"robots\" \
         content=\"noindex\"><title>{}</title></head><body><main>{body}</main><\
         /body></html>",
        escape(title)
    )
}

fn password_form(retry: bool) -> String {
    let notice = if retry {
        "<p role=\"alert\">That password is not right. Try again.</p>"
    } else {
        ""
    };
    document(
        "Password required",
        &format!(
            "<h1>This page is protected</h1>{notice}<form \
             method=\"post\"><label for=\"password\">Password</label> <input \
             id=\"password\" name=\"password\" type=\"password\" \
             autocomplete=\"current-password\" required autofocus> <button \
             type=\"submit\">Open</button></form>"
        ),
    )
}

fn html_response(
    status: StatusCode,
    html: String,
    max_age: u32,
    shared: bool,
) -> Response {
    let mut response = (status, html).into_response();
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let cache = if max_age == 0 || shared {
        "private, no-store".to_owned()
    } else {
        format!("public, max-age={max_age}")
    };
    if let Ok(value) = HeaderValue::from_str(&cache) {
        let _ = headers.insert(header::CACHE_CONTROL, value);
    }
    if shared {
        // The token is in the URL: never leak it to linked sites, never index
        // it.
        let _ = headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        let _ = headers.insert(
            "x-robots-tag",
            HeaderValue::from_static("noindex, nofollow"),
        );
    }
    response
}

fn render(result: Result<PublicPage, Error>, shared: bool) -> Response {
    match result {
        Ok(PublicPage::Html { html, max_age }) => {
            html_response(StatusCode::OK, html, max_age, shared)
        }
        Ok(PublicPage::PasswordRequired { retry }) => {
            let status = if retry {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            };
            html_response(status, password_form(retry), 0, true)
        }
        Err(error) => {
            let status = crate::error::status_of(error.kind());
            let message = if error.kind() == ErrorKind::Internal {
                "Something went wrong."
            } else {
                error.message()
            };
            html_response(
                status,
                document("Vellum", &format!("<h1>{}</h1>", escape(message))),
                0,
                shared,
            )
        }
    }
}

fn parse_token(token: &str) -> Result<ShareToken, Error> {
    token.parse::<ShareToken>().map_err(Error::from)
}

async fn open_share(
    State(site): State<Arc<dyn PublicSite>>,
    Path(token): Path<String>,
) -> Response {
    let result = match parse_token(&token) {
        Ok(token) => site.shared(token, None).await,
        Err(e) => Err(e),
    };
    render(result, true)
}

/// The password form's body.
#[derive(Debug, Deserialize)]
struct PasswordForm {
    password: String,
}

async fn unlock_share(
    State(site): State<Arc<dyn PublicSite>>,
    Path(token): Path<String>,
    Form(form): Form<PasswordForm>,
) -> Response {
    let result = match parse_token(&token) {
        Ok(token) => site.shared(token, Some(form.password)).await,
        Err(e) => Err(e),
    };
    render(result, true)
}

async fn published(
    State(site): State<Arc<dyn PublicSite>>,
    Path(path): Path<String>,
) -> Response {
    render(site.published(&path).await, false)
}

async fn not_found() -> Response {
    render(
        Err(Error::new(ErrorKind::NotFound, "page not found")),
        false,
    )
}

/// The public origin's routes.
pub fn router(site: Arc<dyn PublicSite>) -> Router {
    Router::new()
        .route("/s/{token}", get(open_share).post(unlock_share))
        .route("/p/{*path}", get(published))
        .fallback(not_found)
        .with_state(site)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_is_escaped() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert!(password_form(true).contains("role=\"alert\""));
        assert!(!password_form(false).contains("alert"));
        assert!(!password_form(false).contains("<script"));
    }

    #[test]
    fn shared_responses_are_private() {
        let r = render(
            Ok(PublicPage::Html {
                html: "x".into(),
                max_age: 600,
            }),
            true,
        );
        assert_eq!(
            r.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("private, no-store")
        );
        assert_eq!(
            r.headers()
                .get(header::REFERRER_POLICY)
                .and_then(|v| v.to_str().ok()),
            Some("no-referrer")
        );
        let r = render(
            Ok(PublicPage::Html {
                html: "x".into(),
                max_age: 600,
            }),
            false,
        );
        assert_eq!(
            r.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("public, max-age=600")
        );
        let r = render(
            Err(Error::new(ErrorKind::Internal, "secret detail")),
            false,
        );
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let r = render(Ok(PublicPage::PasswordRequired { retry: true }), true);
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert!(parse_token("short").is_err());
    }
}
