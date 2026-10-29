// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `X-Request-Id` header.

use http::{HeaderMap, HeaderName};
use vellum_kernel::RequestId;

/// The request-ID header, read from trusted callers and always returned.
pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Reuses a well-formed incoming request ID (a retry from our own client,
/// or a proxy that set one) or generates a fresh one.
#[must_use]
pub fn from_headers(headers: &HeaderMap) -> RequestId {
    headers
        .get(&REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<RequestId>().ok())
        .unwrap_or_else(RequestId::generate)
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    #[test]
    fn reuses_valid_ids_only() {
        let id = RequestId::generate();
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            REQUEST_ID,
            HeaderValue::from_str(&id.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("x")),
        );
        assert_eq!(from_headers(&headers), id);
        let _ = headers.insert(REQUEST_ID, HeaderValue::from_static("<script>"));
        assert_ne!(from_headers(&headers).to_string(), "<script>");
    }
}
