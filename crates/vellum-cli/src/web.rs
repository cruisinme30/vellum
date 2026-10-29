// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The web app, embedded in the binary by `build.rs` and served on the app
//! host for every path the gateway does not own
//! ([`GatewayBuilder::web_app`](vellum_api::gateway::GatewayBuilder::web_app)).
//!
//! - A file of the build (`/assets/index-3f2a.js`, `/sw.js`,
//!   `/manifest.webmanifest`) is served as is, with its content type.
//!   Hashed files under `/assets/` are cached for a year; everything else
//!   is revalidated, and the index is never stored.
//! - A missing file under `/assets/`, or any path whose last segment has a
//!   file extension, is not found: the index would only fail to parse as a
//!   script or image.
//! - Every other `GET` (`/`, `/p/…`, `/data/…`, `/sign-in`) is an app
//!   route: the index, which the app's router takes from there.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::{IntoResponse as _, Response};
use http::{HeaderValue, Method, StatusCode, header};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/web_app.rs"));
}

/// Whether the real web app is embedded, not the "not built" placeholder.
pub const BUILT: bool = generated::BUILT;

/// The embedded web app's files.
pub fn embedded() -> &'static [(&'static str, &'static [u8])] {
    generated::FILES
}

const INDEX: &str = "/index.html";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";
const NEVER_STORE: &str = "no-store";

/// The files, by URL path.
struct Files(HashMap<&'static str, &'static [u8]>);

/// The router serving `files` (URL path and bytes), normally
/// [`embedded`].
pub fn router(files: &'static [(&'static str, &'static [u8])]) -> Router {
    let files = Arc::new(Files(files.iter().copied().collect()));
    Router::new().fallback(serve).with_state(files)
}

/// The content type for a file name, by extension.
fn content_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

/// How long browsers may keep a file.
fn cache_control(path: &str) -> &'static str {
    if path == INDEX {
        NEVER_STORE
    } else if path.starts_with("/assets/") {
        IMMUTABLE
    } else {
        REVALIDATE
    }
}

fn file(path: &str, bytes: &'static [u8]) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    let _ = headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    let _ = headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control(path)),
    );
    response
}

fn not_found() -> Response {
    let mut response = (StatusCode::NOT_FOUND, "not found").into_response();
    let _ = response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(REVALIDATE));
    response
}

/// Whether the path's last segment names a file (`/favicon.ico`).
fn names_a_file(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

async fn serve(State(files): State<Arc<Files>>, request: Request) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let mut response = (StatusCode::METHOD_NOT_ALLOWED, "method not allowed").into_response();
        let _ = response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return response;
    }
    let path = request.uri().path();
    if let Some((&name, &bytes)) = files.0.get_key_value(path) {
        return file(name, bytes);
    }
    if path.starts_with("/assets/") || names_a_file(path) {
        return not_found();
    }
    files
        .0
        .get(INDEX)
        .map_or_else(not_found, |&index| file(INDEX, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_always_has_an_index() {
        assert!(embedded().iter().any(|(path, _)| *path == INDEX));
    }

    #[test]
    fn types_and_lifetimes() {
        assert_eq!(
            content_type("/assets/a-1.JS"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type("/manifest.webmanifest"),
            "application/manifest+json"
        );
        assert_eq!(content_type("/fonts/x.woff2"), "font/woff2");
        assert_eq!(content_type("/LICENSE"), "application/octet-stream");
        assert_eq!(cache_control("/assets/a-1.js"), IMMUTABLE);
        assert_eq!(cache_control("/index.html"), NEVER_STORE);
        assert_eq!(cache_control("/sw.js"), REVALIDATE);
        assert!(names_a_file("/favicon.ico"));
        assert!(!names_a_file("/p/0193a1b2"));
    }
}
