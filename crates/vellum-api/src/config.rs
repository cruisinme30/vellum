// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Gateway settings.

use std::time::Duration;

/// How the gateway behaves at the edge. Built by the binary from the server
/// configuration; every field has a safe default.
#[derive(Clone)]
pub struct GatewayConfig {
    /// The app's external origin (`https://vellum.northstar.dev`). Unsafe
    /// requests carrying an `Origin` header must match it.
    pub app_origin: String,
    /// The separate origin serving public pages and share links
    /// (`https://pages.northstar.dev`). `None` disables public pages.
    pub public_origin: Option<String>,
    /// Whether the app is served over HTTPS (directly or behind a proxy),
    /// which turns on HSTS and `Secure` cookies.
    pub https: bool,
    /// Key for CSRF tokens. At least 32 random bytes, stable across
    /// restarts and replicas.
    pub csrf_key: Vec<u8>,
    /// Largest accepted JSON request body.
    pub max_body_bytes: usize,
    /// Requests per second allowed per caller (user, token or IP).
    pub rate_per_second: u32,
    /// Requests a caller may make at once before the steady rate applies.
    pub rate_burst: u32,
    /// How long a stored idempotent response is replayed.
    pub idempotency_ttl: Duration,
    /// How long shutdown waits for in-flight requests.
    pub drain_timeout: Duration,
    /// Extra origins allowed in `frame-src` for sandboxed embeds.
    pub embed_origins: Vec<String>,
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("app_origin", &self.app_origin)
            .field("public_origin", &self.public_origin)
            .field("https", &self.https)
            .field("csrf_key", &"<redacted>")
            .field("max_body_bytes", &self.max_body_bytes)
            .field("rate_per_second", &self.rate_per_second)
            .field("rate_burst", &self.rate_burst)
            .field("idempotency_ttl", &self.idempotency_ttl)
            .field("drain_timeout", &self.drain_timeout)
            .field("embed_origins", &self.embed_origins)
            .finish()
    }
}

impl GatewayConfig {
    /// Defaults for an app served at `app_origin`, with a CSRF key.
    #[must_use]
    pub fn new(app_origin: impl Into<String>, csrf_key: Vec<u8>) -> Self {
        let app_origin = app_origin.into().trim_end_matches('/').to_owned();
        let https = app_origin.starts_with("https://");
        Self {
            app_origin,
            public_origin: None,
            https,
            csrf_key,
            max_body_bytes: 10 * 1024 * 1024,
            rate_per_second: 50,
            rate_burst: 200,
            idempotency_ttl: Duration::from_hours(24),
            drain_timeout: Duration::from_secs(30),
            embed_origins: Vec::new(),
        }
    }

    /// The host (and port) of the public-pages origin, lowercased.
    #[must_use]
    pub fn public_host(&self) -> Option<String> {
        self.public_origin.as_deref().map(host_of)
    }

    /// The host (and port) of the app origin, lowercased.
    #[must_use]
    pub fn app_host(&self) -> String {
        host_of(&self.app_origin)
    }
}

/// The `host[:port]` part of an origin, lowercased.
#[must_use]
pub fn host_of(origin: &str) -> String {
    let rest = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    rest.split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let mut c =
            GatewayConfig::new("https://Vellum.Northstar.dev/", vec![0; 32]);
        c.public_origin = Some("https://pages.northstar.dev:8443".into());
        assert_eq!(c.app_origin, "https://Vellum.Northstar.dev");
        assert_eq!(c.app_host(), "vellum.northstar.dev");
        assert_eq!(
            c.public_host().as_deref(),
            Some("pages.northstar.dev:8443")
        );
        assert!(c.https);
        assert!(!format!("{c:?}").contains("[0"));
        assert!(!GatewayConfig::new("http://localhost:7700", vec![]).https);
    }
}
