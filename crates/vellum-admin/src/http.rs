// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP client the admin commands use to ask a running server.

use std::time::Duration;

use serde::de::DeserializeOwned;
use vellum_kernel::{Error, ErrorKind, Result};

/// Sends `GET {server}{path}?{query}` with an optional bearer token and
/// decodes the JSON answer.
///
/// # Errors
///
/// Returns invalid input for a bad server URL, unauthenticated or
/// permission denied for 401 and 403, and unavailable for anything else
/// that goes wrong.
pub fn get_json<T: DeserializeOwned>(
    server: &str,
    token: Option<&str>,
    path: &str,
    query: &[(&str, &str)],
) -> Result<T> {
    let unavailable = |what: &'static str| {
        move |e: reqwest::Error| {
            Error::new(ErrorKind::Unavailable, what).with_source(e)
        }
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(unavailable("cannot build the HTTP client"))?;
    let mut url =
        reqwest::Url::parse(&format!("{}{path}", server.trim_end_matches('/')))
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "the server URL is not valid",
                )
                .with_source(e)
            })?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            let _ = pairs.append_pair(key, value);
        }
    }
    let mut request = client.get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .map_err(unavailable("cannot reach the server"))?;
    let status = response.status();
    if !status.is_success() {
        let kind = match status.as_u16() {
            401 => ErrorKind::Unauthenticated,
            403 => ErrorKind::PermissionDenied,
            _ => ErrorKind::Unavailable,
        };
        return Err(Error::new(
            kind,
            format!("the server answered {}", status.as_u16()),
        ));
    }
    response
        .json()
        .map_err(unavailable("the server's answer is not what was expected"))
}
