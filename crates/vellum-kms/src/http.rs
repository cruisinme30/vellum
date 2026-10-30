// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Pieces every remote provider shares: the HTTP client, JSON and base64
//! handling, associated data, error mapping and telemetry.

use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use vellum_crypto::{CryptoError, KekId, SecretKey};

/// How long one call to a key service may take, connect included.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Prefix of the associated data bound into every wrap, as for local KEKs.
const AAD_PREFIX: &str = "vellum/dek/v1:";

/// A client with the shared timeout. No redirects: a key service that
/// redirects is misconfigured, and following one could leak a token.
pub fn client() -> Result<reqwest::Client, CryptoError> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| CryptoError::Provider(format!("cannot build HTTP client: {e}")))
}

/// The associated data for a DEK wrapped under `kek`.
pub fn aad(kek: &KekId) -> String {
    format!("{AAD_PREFIX}{kek}")
}

pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn unb64(backend: &str, text: &str) -> Result<Vec<u8>, CryptoError> {
    STANDARD
        .decode(text)
        .map_err(|_| CryptoError::Provider(format!("{backend}: response is not valid base64")))
}

/// Reads the string at `pointer` (a JSON pointer) from a response body.
pub fn string_at<'a>(
    backend: &str,
    body: &'a Value,
    pointer: &str,
) -> Result<&'a str, CryptoError> {
    body.pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| CryptoError::Provider(format!("{backend}: response lacks {pointer}")))
}

/// Turns a decoded key from a service into a DEK.
pub fn dek(backend: &str, bytes: &[u8]) -> Result<SecretKey, CryptoError> {
    SecretKey::from_slice(bytes).map_err(|_| {
        CryptoError::Provider(format!("{backend}: unwrapped key has the wrong length"))
    })
}

/// Encodes a request body. A wrap request holds the DEK in base64; the HTTP
/// stack keeps its own copy, so this is not zeroized either.
pub fn to_json(backend: &str, body: &Value) -> Result<Vec<u8>, CryptoError> {
    serde_json::to_vec(body)
        .map_err(|_| CryptoError::Provider(format!("{backend}: cannot encode request")))
}

/// Sends a request and parses a JSON body, mapping transport failures and
/// non-2xx statuses to [`CryptoError::Provider`]. Error bodies are cut short
/// and never contain key material (services do not echo plaintext).
pub async fn send_json(
    backend: &str,
    request: reqwest::RequestBuilder,
) -> Result<Value, CryptoError> {
    let response = request.send().await.map_err(|e| {
        CryptoError::Provider(format!("{backend}: request failed: {}", without_url(&e)))
    })?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|e| {
        CryptoError::Provider(format!(
            "{backend}: reading response failed: {}",
            without_url(&e)
        ))
    })?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        let short: String = text.chars().take(200).collect();
        return Err(CryptoError::Provider(format!(
            "{backend}: HTTP {status}: {short}"
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CryptoError::Provider(format!("{backend}: response is not JSON")))
}

/// Sends a request and returns a short text body (instance metadata).
/// Non-2xx statuses become [`CryptoError::Provider`] without the body.
pub async fn send_text(
    backend: &str,
    request: reqwest::RequestBuilder,
) -> Result<String, CryptoError> {
    let response = request.send().await.map_err(|e| {
        CryptoError::Provider(format!("{backend}: request failed: {}", without_url(&e)))
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(CryptoError::Provider(format!("{backend}: HTTP {status}")));
    }
    response.text().await.map_err(|e| {
        CryptoError::Provider(format!(
            "{backend}: reading response failed: {}",
            without_url(&e)
        ))
    })
}

/// Error text without the request URL, which can name keys and accounts.
fn without_url(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    if let Some(url) = error.url() {
        text = text.replace(url.as_str(), "<url>");
    }
    text
}

/// Counts one call and records its latency, then passes the result through.
pub fn record<T>(
    backend: &'static str,
    op: &'static str,
    started: Instant,
    result: Result<T, CryptoError>,
) -> Result<T, CryptoError> {
    let outcome = if result.is_ok() { "ok" } else { "error" };
    tracing::info!(
        monotonic_counter.vellum_kms_requests = 1_u64,
        histogram.vellum_kms_request_seconds = started.elapsed().as_secs_f64(),
        backend,
        op,
        outcome,
    );
    if let Err(error) = &result {
        tracing::warn!(backend, op, %error, "key service call failed");
    }
    result
}

/// True when every character is one of `[A-Za-z0-9_.-]` and `extra`, and
/// the text is non-empty and not a path traversal.
pub fn safe_segment(text: &str, extra: &[char]) -> bool {
    !text.is_empty()
        && !text
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') || extra.contains(&c)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_reject_traversal_and_odd_characters() {
        assert!(safe_segment("transit", &[]));
        assert!(safe_segment("team/transit", &['/']));
        assert!(!safe_segment("", &[]));
        assert!(!safe_segment("a/../b", &['/']));
        assert!(!safe_segment("a//b", &['/']));
        assert!(!safe_segment("a/b", &[]));
        assert!(!safe_segment("a?b", &[]));
        assert!(!safe_segment("a b", &[]));
    }
}
