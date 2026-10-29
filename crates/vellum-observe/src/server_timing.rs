// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `Server-Timing` response header (brief §12: "on every response").
//!
//! Code on the request path adds named durations with [`record`] or
//! [`measure`]; the HTTP middleware collects them, adds `total` and the
//! `traceparent`, and writes the header. Browsers expose the header through
//! the Resource Timing API, which is how client telemetry learns the server
//! trace id of a navigation.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::future::Future;
use std::time::{Duration, Instant};

/// One metric in the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metric {
    /// The metric name: a header token such as `db` or `authz`.
    pub name: String,
    /// Total duration recorded under this name.
    pub duration: Option<Duration>,
    /// An optional description.
    pub description: Option<String>,
}

tokio::task_local! {
    static METRICS: RefCell<Vec<Metric>>;
}

/// Runs `fut` with a fresh collector and returns its output with the
/// metrics recorded inside it.
pub async fn collect<F: Future>(fut: F) -> (F::Output, Vec<Metric>) {
    METRICS
        .scope(RefCell::new(Vec::new()), async {
            let out = fut.await;
            let metrics = METRICS.with(std::cell::RefCell::take);
            (out, metrics)
        })
        .await
}

/// Adds `duration` under `name` to the current request's header. Several
/// records with one name are summed. Outside a request it does nothing.
pub fn record(name: &str, duration: Duration) {
    let name = token(name);
    let _ = METRICS.try_with(|m| {
        let mut metrics = m.borrow_mut();
        if let Some(existing) = metrics.iter_mut().find(|x| x.name == name) {
            existing.duration = Some(existing.duration.unwrap_or_default() + duration);
        } else {
            metrics.push(Metric {
                name,
                duration: Some(duration),
                description: None,
            });
        }
    });
}

/// Times `fut` and records it under `name`.
pub async fn measure<F: Future>(name: &str, fut: F) -> F::Output {
    let start = Instant::now();
    let out = fut.await;
    record(name, start.elapsed());
    out
}

/// Keeps only header token characters (RFC 9110), so a metric name can
/// never break the header.
fn token(name: &str) -> String {
    let t: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(*c))
        .take(64)
        .collect();
    if t.is_empty() {
        "unnamed".to_owned()
    } else {
        t
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars().filter(|c| !c.is_control()).take(200) {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Formats metrics as a `Server-Timing` header value.
#[must_use]
pub fn header_value(metrics: &[Metric]) -> String {
    let mut out = String::new();
    for m in metrics {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&token(&m.name));
        if let Some(d) = m.duration {
            let _ = write!(out, ";dur={:.1}", d.as_secs_f64() * 1000.0);
        }
        if let Some(desc) = &m.description {
            let _ = write!(out, ";desc={}", quoted(desc));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collects_and_sums() {
        let ((), metrics) = collect(async {
            record("db", Duration::from_millis(2));
            record("db", Duration::from_millis(3));
            measure("authz", async {}).await;
            record("bad name\r\n", Duration::ZERO);
        })
        .await;
        assert_eq!(metrics.len(), 3);
        let db = metrics.first().unwrap();
        assert_eq!(db.duration, Some(Duration::from_millis(5)));
        assert_eq!(metrics.get(2).unwrap().name, "badname");
    }

    #[test]
    fn record_outside_a_request_is_ignored() {
        record("db", Duration::from_millis(1));
    }

    #[test]
    fn formats_the_header() {
        let v = header_value(&[
            Metric {
                name: "total".into(),
                duration: Some(Duration::from_micros(12_340)),
                description: None,
            },
            Metric {
                name: "traceparent".into(),
                duration: None,
                description: Some("00-a\"b\\c".into()),
            },
            Metric {
                name: String::new(),
                duration: None,
                description: None,
            },
        ]);
        assert_eq!(
            v,
            r#"total;dur=12.3, traceparent;desc="00-a\"b\\c", unnamed"#
        );
    }
}
