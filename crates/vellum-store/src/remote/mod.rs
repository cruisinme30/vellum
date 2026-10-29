// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Plumbing shared by the HTTP adapters (S3, Google Drive, Dropbox,
//! `OneDrive`): one client with timeouts and bounded retries, bearer-token
//! sources, and the mapping from HTTP status to [`StoreError`].
//!
//! Each adapter is a module behind its own cargo feature, so a deployment
//! compiles only the backends it uses.

#[cfg(feature = "dropbox")]
pub mod dropbox;
#[cfg(feature = "gdrive")]
pub mod gdrive;
mod oauth;
#[cfg(feature = "onedrive")]
pub mod onedrive;

use std::{fmt, time::Duration};

use bytes::Bytes;
use reqwest::{RequestBuilder, Response, StatusCode, header::RETRY_AFTER};

#[cfg(feature = "dropbox")]
pub use dropbox::{DropboxConfig, DropboxStore};
pub use oauth::{RefreshToken, StaticToken, TokenSource};

use crate::{
    error::{StoreError, StoreResult},
    path::BlobPath,
};

/// Attempts per request, including the first.
const MAX_ATTEMPTS: u32 = 4;
/// First retry delay; doubles on each retry.
const BACKOFF: Duration = Duration::from_millis(200);
/// Longest `Retry-After` honoured before giving up instead.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// An HTTP client for one backend: timeouts, retries on throttling and
/// server errors, and spans named after the backend.
#[derive(Clone, Debug)]
pub struct Http {
    client: reqwest::Client,
    backend: &'static str,
}

impl Http {
    /// Builds a client for `backend` with connect and request timeouts.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if the TLS stack cannot start.
    pub fn new(backend: &'static str) -> StoreResult<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_mins(2))
            .user_agent(concat!("vellum/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| StoreError::Backend(format!("{backend}: cannot build client: {e}")))?;
        Ok(Self { client, backend })
    }

    /// The underlying client, for building requests.
    #[must_use]
    pub const fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Sends the request `build` makes, retrying transport errors, `429`
    /// and `5xx` with exponential backoff (or the server's `Retry-After`).
    /// `build` runs once per attempt, so bodies must be cheap to clone,
    /// such as [`Bytes`].
    ///
    /// Any response that is not retried is returned, whatever its status.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] once the attempts are used up.
    pub async fn send(&self, build: impl Fn() -> RequestBuilder) -> StoreResult<Response> {
        let mut delay = BACKOFF;
        let mut attempt = 1;
        loop {
            let outcome = build().send().await;
            let wait = match &outcome {
                Ok(r) if retryable(r.status()) => Some(retry_after(r).unwrap_or(delay)),
                Err(e) if !e.is_builder() => Some(delay),
                _ => None,
            };
            match wait {
                Some(wait) if attempt < MAX_ATTEMPTS && wait <= MAX_RETRY_AFTER => {
                    tracing::debug!(backend = self.backend, attempt, ?wait, "retrying request");
                    tokio::time::sleep(wait).await;
                    delay = delay.saturating_mul(2);
                    attempt += 1;
                }
                _ => {
                    return outcome.map_err(|e| {
                        StoreError::Backend(format!("{}: {}", self.backend, redact(e)))
                    });
                }
            }
        }
    }
}

const fn retryable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

fn retry_after(response: &Response) -> Option<Duration> {
    let secs: u64 = response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs))
}

/// Transport errors can carry the request URL, which may hold a signed
/// query string; keep only the error kind.
fn redact(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

/// Turns a non-success response into a [`StoreError`].
///
/// `404` is [`StoreError::NotFound`], `412` is
/// [`StoreError::PreconditionFailed`], and everything else is [`StoreError::Backend`] with the status and the
/// start of the body (never headers, which may hold credentials).
pub async fn status_error(backend: &str, path: &BlobPath, response: Response) -> StoreError {
    let status = response.status();
    match status {
        StatusCode::NOT_FOUND => StoreError::NotFound(path.clone()),
        StatusCode::PRECONDITION_FAILED => StoreError::PreconditionFailed {
            path: path.clone(),
            current: None,
        },
        _ => {
            let body = response.bytes().await.unwrap_or_default();
            backend_error(backend, path, status, &body)
        }
    }
}

/// A [`StoreError::Backend`] naming the status and a short body excerpt.
#[must_use]
pub fn backend_error(
    backend: &str,
    path: &BlobPath,
    status: StatusCode,
    body: &Bytes,
) -> StoreError {
    let excerpt = String::from_utf8_lossy(body.get(..body.len().min(300)).unwrap_or_default());
    StoreError::Backend(format!("{backend}: {status} on {path}: {}", excerpt.trim()))
}

/// A secret that never appears in `Debug` output or logs.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, for signing or a header.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn retries_only_throttling_and_server_errors() {
        assert!(retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!retryable(StatusCode::NOT_FOUND));
        assert!(!retryable(StatusCode::PRECONDITION_FAILED));
        assert!(!retryable(StatusCode::NOT_IMPLEMENTED));
    }
}
