// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Client for the Kroki sidecar (`D2`, PlantUML, Graphviz, Excalidraw).
//!
//! Every call has a timeout, and a circuit breaker stops calling a Kroki
//! that keeps failing, so a dead sidecar costs one timeout per cool-down,
//! not one per diagram. Callers fall back to the last good SVG.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// After this many consecutive failures the breaker opens.
const TRIP_AFTER: u32 = 3;
/// How long an open breaker refuses calls before letting one through.
const COOL_DOWN: Duration = Duration::from_secs(30);
/// Kroki answers larger than this are refused.
const MAX_SVG: usize = 4 * 1024 * 1024;

/// Why Kroki could not render a diagram.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KrokiError {
    /// The diagram source is invalid; Kroki's message says why.
    #[error("{0}")]
    Source(String),
    /// Kroki is unreachable, slow or failing, or the breaker is open.
    #[error("diagram service unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Default)]
struct Breaker {
    failures: u32,
    open_until: Option<Instant>,
}

/// A Kroki HTTP client.
#[derive(Debug)]
pub struct Kroki {
    base: String,
    http: reqwest::Client,
    breaker: Mutex<Breaker>,
}

/// Kroki's endpoint name for a fence language, or `None` when Kroki does
/// not draw it.
#[must_use]
pub fn endpoint(lang: &str) -> Option<&'static str> {
    match lang {
        "d2" => Some("d2"),
        "plantuml" => Some("plantuml"),
        "graphviz" | "dot" => Some("graphviz"),
        "excalidraw" => Some("excalidraw"),
        _ => None,
    }
}

impl Kroki {
    /// A client for the Kroki at `base` (such as `http://kroki:8000`).
    ///
    /// # Errors
    ///
    /// Returns a message when the HTTP client cannot be built.
    pub fn new(base: &str, timeout: Duration) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("vellum-render/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            http,
            breaker: Mutex::new(Breaker::default()),
        })
    }

    fn allow(&self) -> bool {
        let Ok(mut b) = self.breaker.lock() else {
            return true;
        };
        match b.open_until {
            Some(until) if Instant::now() < until => false,
            Some(_) => {
                // Half-open: let this call probe.
                b.open_until = None;
                true
            }
            None => true,
        }
    }

    fn record(&self, ok: bool) {
        let Ok(mut b) = self.breaker.lock() else {
            return;
        };
        if ok {
            b.failures = 0;
        } else {
            b.failures += 1;
            if b.failures >= TRIP_AFTER {
                b.open_until = Some(Instant::now() + COOL_DOWN);
                b.failures = 0;
            }
        }
    }

    /// True while the breaker refuses calls.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.breaker
            .lock()
            .is_ok_and(|b| b.open_until.is_some_and(|u| Instant::now() < u))
    }

    /// Renders `source` with Kroki's `kind` endpoint and returns the SVG.
    ///
    /// # Errors
    ///
    /// [`KrokiError::Source`] when Kroki rejects the source (HTTP 400), and
    /// [`KrokiError::Unavailable`] for everything else.
    #[tracing::instrument(name = "render.kroki", skip_all, fields(kind, bytes = source.len(), status))]
    pub async fn render(&self, kind: &str, source: &str) -> Result<String, KrokiError> {
        if !self.allow() {
            return Err(KrokiError::Unavailable(
                "circuit open after repeated failures".to_owned(),
            ));
        }
        let url = format!("{}/{kind}/svg", self.base);
        let response = self
            .http
            .post(&url)
            .header("content-type", "text/plain; charset=utf-8")
            .header("accept", "image/svg+xml")
            .body(source.to_owned())
            .send()
            .await;
        let response = match response {
            Ok(r) => r,
            Err(e) => {
                self.record(false);
                return Err(KrokiError::Unavailable(e.without_url().to_string()));
            }
        };
        let status = response.status();
        let _ = tracing::Span::current().record("status", status.as_u16());
        let body = response.text().await;
        match (status.as_u16(), body) {
            (200, Ok(svg)) if svg.len() <= MAX_SVG => {
                self.record(true);
                Ok(svg)
            }
            (200, Ok(_)) => {
                self.record(true);
                Err(KrokiError::Source("diagram is too large".to_owned()))
            }
            (400, Ok(message)) => {
                // The service works; the source is wrong.
                self.record(true);
                let first = message
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("invalid diagram");
                Err(KrokiError::Source(first.chars().take(300).collect()))
            }
            (code, _) => {
                self.record(false);
                Err(KrokiError::Unavailable(format!("HTTP {code}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    /// A one-response-per-connection HTTP server that replies with `status`
    /// and `body`.
    async fn serve(status: u16, body: &'static str, connections: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            for _ in 0..connections {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: image/svg+xml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn returns_svg() {
        let base = serve(200, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>", 1).await;
        let k = Kroki::new(&base, Duration::from_secs(5)).unwrap();
        assert_eq!(
            k.render("graphviz", "digraph{a->b}").await.unwrap(),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>"
        );
    }

    #[tokio::test]
    async fn reports_source_errors_without_tripping() {
        let base = serve(400, "Error 400: syntax error in line 1\nmore", 4).await;
        let k = Kroki::new(&base, Duration::from_secs(5)).unwrap();
        for _ in 0..4 {
            assert_eq!(
                k.render("d2", "x ->").await,
                Err(KrokiError::Source(
                    "Error 400: syntax error in line 1".into()
                ))
            );
        }
        assert!(!k.is_open());
    }

    #[tokio::test]
    async fn trips_the_breaker_when_down() {
        // Nothing listens on this port once the listener is dropped.
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let k = Kroki::new(&format!("http://{addr}"), Duration::from_millis(500)).unwrap();
        for _ in 0..TRIP_AFTER {
            assert!(matches!(
                k.render("d2", "a -> b").await,
                Err(KrokiError::Unavailable(_))
            ));
        }
        assert!(k.is_open());
        assert_eq!(
            k.render("d2", "a -> b").await,
            Err(KrokiError::Unavailable(
                "circuit open after repeated failures".into()
            ))
        );
    }

    #[test]
    fn maps_languages() {
        assert_eq!(endpoint("graphviz"), Some("graphviz"));
        assert_eq!(endpoint("mermaid"), None);
        assert_eq!(endpoint("drawio"), None);
    }
}
