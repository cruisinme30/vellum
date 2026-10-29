// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Client telemetry joined to server traces.
//!
//! The web app learns the server trace id of each navigation or API call
//! from `Server-Timing` (see [`crate::server_timing`]) and posts what the
//! browser measured — navigation phases, Core Web Vitals, slow fetches —
//! back with that id. [`ClientIngest`] validates the report, joins it to the
//! trace in the ring buffer, exports the spans over OpenTelemetry under the
//! same trace id, and records the vitals as histograms.
//!
//! Reports are untrusted input: sizes, counts and numbers are bounded, and
//! page paths are reduced to templates so metric labels stay bounded.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::KeyValue;
use opentelemetry::metrics::Histogram;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanKind, TraceContextExt as _, TraceFlags, TraceState, Tracer as _,
};
use serde::{Deserialize, Serialize};

use crate::store::{ClientSpan, TraceStore};
use crate::trace_id::TraceParent;

/// The largest report body accepted, in bytes.
pub const MAX_REPORT_BYTES: usize = 64 * 1024;
/// The most spans in one report.
pub const MAX_SPANS: usize = 64;
/// The most attributes on one span.
pub const MAX_ATTRIBUTES: usize = 16;
const MAX_NAME: usize = 128;
const MAX_KEY: usize = 64;
const MAX_VALUE: usize = 256;
/// The longest duration or offset accepted: one hour, in milliseconds.
const MAX_MS: f64 = 3_600_000.0;

/// Core Web Vitals and related page metrics, in milliseconds except CLS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vitals {
    /// Largest Contentful Paint.
    pub lcp_ms: Option<f64>,
    /// Interaction to Next Paint.
    pub inp_ms: Option<f64>,
    /// Cumulative Layout Shift (unitless).
    pub cls: Option<f64>,
    /// Time to First Byte.
    pub ttfb_ms: Option<f64>,
    /// First Contentful Paint.
    pub fcp_ms: Option<f64>,
}

/// One span measured in the browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedSpan {
    /// What was measured.
    pub name: String,
    /// Start, in milliseconds on the page's performance time origin.
    pub start_ms: f64,
    /// Duration in milliseconds.
    pub duration_ms: f64,
    /// Extra string attributes.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
}

/// A report posted by the web app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientReport {
    /// The server trace this report belongs to, from `Server-Timing`.
    pub traceparent: String,
    /// The page path; reduced to a template before use.
    pub page: String,
    /// When the traced request was sent, on the page's time origin, in ms.
    pub request_start_ms: f64,
    /// `performance.timeOrigin`, in Unix milliseconds.
    pub time_origin_ms: f64,
    /// Spans measured in the browser.
    #[serde(default)]
    pub spans: Vec<ReportedSpan>,
    /// Page vitals, when this report closes a page view.
    #[serde(default)]
    pub vitals: Vitals,
}

/// Why a report was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    /// The body is larger than [`MAX_REPORT_BYTES`].
    #[error("report is too large")]
    TooLarge,
    /// The body is not a valid report.
    #[error("malformed report: {0}")]
    Malformed(String),
    /// A field is out of range.
    #[error("invalid report: {0}")]
    Invalid(&'static str),
}

/// What happened to an accepted report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ingested {
    /// Spans accepted.
    pub spans: usize,
    /// Whether the server trace was still in the ring buffer and was joined.
    pub joined: bool,
}

fn valid_ms(v: f64, allow_negative: bool) -> bool {
    v.is_finite() && v <= MAX_MS && (allow_negative || v >= 0.0)
}

fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Reduces a path to a template: query and fragment removed, at most four
/// segments, and segments that look like identifiers replaced by `{id}`.
#[must_use]
pub fn page_template(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .take(4)
        .map(|s| {
            let looks_like_id = s.len() > 24
                || s.chars().any(|c| c.is_ascii_digit())
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if looks_like_id {
                "{id}".to_owned()
            } else {
                s.to_ascii_lowercase()
            }
        })
        .collect();
    format!("/{}", segments.join("/"))
}

impl ClientReport {
    /// Parses and validates a JSON body.
    ///
    /// # Errors
    ///
    /// Returns [`ReportError`] when the body is too large, malformed or out
    /// of range.
    pub fn parse(body: &[u8]) -> Result<Self, ReportError> {
        if body.len() > MAX_REPORT_BYTES {
            return Err(ReportError::TooLarge);
        }
        let report: Self =
            serde_json::from_slice(body).map_err(|e| ReportError::Malformed(e.to_string()))?;
        report.validate()?;
        Ok(report)
    }

    fn validate(&self) -> Result<(), ReportError> {
        if self.traceparent.parse::<TraceParent>().is_err() {
            return Err(ReportError::Invalid("traceparent"));
        }
        if self.page.len() > 2048 {
            return Err(ReportError::Invalid("page"));
        }
        if !valid_ms(self.request_start_ms, false)
            || !self.time_origin_ms.is_finite()
            || self.time_origin_ms < 0.0
        {
            return Err(ReportError::Invalid("timing"));
        }
        if self.spans.len() > MAX_SPANS {
            return Err(ReportError::Invalid("spans"));
        }
        for span in &self.spans {
            if span.name.is_empty() || span.name.len() > MAX_NAME * 4 {
                return Err(ReportError::Invalid("span name"));
            }
            if !valid_ms(span.start_ms, false) || !valid_ms(span.duration_ms, false) {
                return Err(ReportError::Invalid("span timing"));
            }
            if span.attributes.len() > MAX_ATTRIBUTES {
                return Err(ReportError::Invalid("attributes"));
            }
        }
        let v = self.vitals;
        for ms in [v.lcp_ms, v.inp_ms, v.ttfb_ms, v.fcp_ms]
            .into_iter()
            .flatten()
        {
            if !valid_ms(ms, false) {
                return Err(ReportError::Invalid("vitals"));
            }
        }
        if v.cls
            .is_some_and(|c| !c.is_finite() || !(0.0..=100.0).contains(&c))
        {
            return Err(ReportError::Invalid("cls"));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct VitalInstruments {
    lcp: Histogram<f64>,
    inp: Histogram<f64>,
    cls: Histogram<f64>,
    ttfb: Histogram<f64>,
    fcp: Histogram<f64>,
}

/// Accepts client reports.
#[derive(Debug, Clone)]
pub struct ClientIngest {
    store: Arc<TraceStore>,
    vitals: Arc<VitalInstruments>,
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "values are validated finite and below one hour"
)]
fn micros(ms: f64) -> i64 {
    (ms * 1000.0).round() as i64
}

impl ClientIngest {
    /// An ingest joining reports to traces in `store`.
    #[must_use]
    pub fn new(store: Arc<TraceStore>) -> Self {
        let meter = opentelemetry::global::meter("vellum-client");
        let ms = |name: &'static str, description: &'static str| {
            meter
                .f64_histogram(name)
                .with_unit("ms")
                .with_description(description)
                .build()
        };
        Self {
            store,
            vitals: Arc::new(VitalInstruments {
                lcp: ms("vellum.client.lcp", "Largest Contentful Paint"),
                inp: ms("vellum.client.inp", "Interaction to Next Paint"),
                ttfb: ms("vellum.client.ttfb", "Time to First Byte"),
                fcp: ms("vellum.client.fcp", "First Contentful Paint"),
                cls: meter
                    .f64_histogram("vellum.client.cls")
                    .with_unit("1")
                    .with_description("Cumulative Layout Shift")
                    .build(),
            }),
        }
    }

    /// Joins, exports and records a validated report.
    #[must_use]
    pub fn ingest(&self, report: &ClientReport) -> Ingested {
        let Ok(parent) = report.traceparent.parse::<TraceParent>() else {
            return Ingested {
                spans: 0,
                joined: false,
            };
        };
        let page = page_template(&report.page);
        let spans: Vec<ClientSpan> = report
            .spans
            .iter()
            .map(|s| {
                let mut attributes: Vec<(String, String)> = s
                    .attributes
                    .iter()
                    .take(MAX_ATTRIBUTES)
                    .map(|(k, v)| (clean(k, MAX_KEY), clean(v, MAX_VALUE)))
                    .collect();
                attributes.push(("page".to_owned(), page.clone()));
                ClientSpan {
                    name: clean(&s.name, MAX_NAME),
                    start_us: micros(s.start_ms - report.request_start_ms),
                    duration_us: u64::try_from(micros(s.duration_ms)).unwrap_or(0),
                    attributes,
                }
            })
            .collect();
        let anchor_ms = self.store.get(parent.trace_id).map_or_else(
            || report.time_origin_ms + report.request_start_ms,
            |t| {
                #[expect(clippy::cast_precision_loss, reason = "Unix milliseconds fit in f64")]
                let ms = t.started_at_ms as f64;
                ms
            },
        );
        export_spans(&parent, anchor_ms, &spans);
        self.record_vitals(&report.vitals, &page);
        let count = spans.len();
        let joined = self.store.attach_client(parent.trace_id, spans);
        Ingested {
            spans: count,
            joined,
        }
    }

    fn record_vitals(&self, v: &Vitals, page: &str) {
        let attrs = [KeyValue::new("page", page.to_owned())];
        let pairs = [
            (&self.vitals.lcp, v.lcp_ms),
            (&self.vitals.inp, v.inp_ms),
            (&self.vitals.cls, v.cls),
            (&self.vitals.ttfb, v.ttfb_ms),
            (&self.vitals.fcp, v.fcp_ms),
        ];
        for (histogram, value) in pairs {
            if let Some(value) = value {
                histogram.record(value, &attrs);
            }
        }
    }
}

/// Exports browser spans over OpenTelemetry as children of the traced
/// request, so they appear in the same trace in Tempo or Jaeger.
fn export_spans(parent: &TraceParent, anchor_ms: f64, spans: &[ClientSpan]) {
    let tracer = opentelemetry::global::tracer("vellum-client");
    let context = SpanContext::new(
        opentelemetry::trace::TraceId::from_bytes(parent.trace_id.to_bytes()),
        opentelemetry::trace::SpanId::from_bytes(parent.parent_id.to_bytes()),
        if parent.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        },
        true,
        TraceState::default(),
    );
    let cx = opentelemetry::Context::new().with_remote_span_context(context);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "anchor is a positive Unix time in milliseconds"
    )]
    let anchor = UNIX_EPOCH + Duration::from_micros((anchor_ms.max(0.0) * 1000.0) as u64);
    for s in spans {
        let start = offset(anchor, s.start_us);
        let end = start + Duration::from_micros(s.duration_us);
        let mut attributes: Vec<KeyValue> = s
            .attributes
            .iter()
            .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
            .collect();
        attributes.push(KeyValue::new("vellum.source", "browser"));
        let mut span = tracer
            .span_builder(s.name.clone())
            .with_kind(SpanKind::Client)
            .with_start_time(start)
            .with_attributes(attributes)
            .start_with_context(&tracer, &cx);
        span.end_with_timestamp(end);
    }
}

fn offset(anchor: SystemTime, us: i64) -> SystemTime {
    let d = Duration::from_micros(us.unsigned_abs());
    if us >= 0 {
        anchor + d
    } else {
        anchor
            .checked_sub(d)
            .filter(|t| *t >= UNIX_EPOCH)
            .unwrap_or(UNIX_EPOCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::trace;

    fn body(traceparent: &str) -> String {
        format!(
            r#"{{"traceparent":"{traceparent}","page":"/p/01HZY3K2M4/edit?x=1",
            "request_start_ms":100.0,"time_origin_ms":1767225600000.0,
            "spans":[{{"name":"navigation","start_ms":90.0,"duration_ms":400.5,
                       "attributes":{{"type":"navigate"}}}}],
            "vitals":{{"lcp_ms":812.0,"cls":0.02,"inp_ms":64.0}}}}"#
        )
    }

    #[test]
    fn templates_pages() {
        assert_eq!(page_template("/p/01HZY3K2M4/edit?x=1"), "/p/{id}/edit");
        assert_eq!(page_template("/Admin/Requests#top"), "/admin/requests");
        assert_eq!(page_template("/a/b/c/d/e/f"), "/a/b/c/d");
        assert_eq!(page_template(""), "/");
        assert_eq!(page_template("/x/hello world"), "/x/{id}");
    }

    #[test]
    fn joins_a_report_to_its_trace() {
        let store = Arc::new(TraceStore::default());
        let t = trace("/api/x", 200, 1000, 1_767_225_600_100);
        let parent = TraceParent {
            trace_id: t.trace_id,
            parent_id: crate::SpanId::random(),
            sampled: true,
        };
        let _ = store.push(t);
        let ingest = ClientIngest::new(Arc::clone(&store));
        let report = ClientReport::parse(body(&parent.to_string()).as_bytes()).unwrap();
        let result = ingest.ingest(&report);
        assert_eq!(
            result,
            Ingested {
                spans: 1,
                joined: true
            }
        );
        let joined = store.get(parent.trace_id).unwrap();
        let span = joined.client.first().unwrap();
        assert_eq!(span.start_us, -10_000);
        assert_eq!(span.duration_us, 400_500);
        assert!(
            span.attributes
                .contains(&("page".into(), "/p/{id}/edit".into()))
        );
    }

    #[test]
    fn unknown_traces_are_exported_but_not_joined() {
        let ingest = ClientIngest::new(Arc::new(TraceStore::default()));
        let report =
            ClientReport::parse(body(&TraceParent::new_root().to_string()).as_bytes()).unwrap();
        assert!(!ingest.ingest(&report).joined);
    }

    #[test]
    fn rejects_bad_reports() {
        assert_eq!(
            ClientReport::parse(&vec![b' '; MAX_REPORT_BYTES + 1]),
            Err(ReportError::TooLarge)
        );
        assert!(matches!(
            ClientReport::parse(b"{"),
            Err(ReportError::Malformed(_))
        ));
        let good = body(&TraceParent::new_root().to_string());
        let cases = [
            good.replace("\"traceparent\":\"00", "\"traceparent\":\"zz"),
            good.replace("\"request_start_ms\":100.0", "\"request_start_ms\":-1.0"),
            good.replace("\"duration_ms\":400.5", "\"duration_ms\":9999999999.0"),
            good.replace("\"name\":\"navigation\"", "\"name\":\"\""),
            good.replace("\"cls\":0.02", "\"cls\":1000.0"),
            good.replace("\"lcp_ms\":812.0", "\"lcp_ms\":-5.0"),
            good.replace(
                "\"time_origin_ms\":1767225600000.0",
                "\"time_origin_ms\":-1.0",
            ),
        ];
        for case in cases {
            assert!(
                matches!(
                    ClientReport::parse(case.as_bytes()),
                    Err(ReportError::Invalid(_))
                ),
                "{case}"
            );
        }
        let unknown = good.replace("\"page\"", "\"extra\":1,\"page\"");
        assert!(matches!(
            ClientReport::parse(unknown.as_bytes()),
            Err(ReportError::Malformed(_))
        ));
        let mut many: ClientReport = serde_json::from_str(&good).unwrap();
        many.spans = vec![many.spans.first().unwrap().clone(); MAX_SPANS + 1];
        assert_eq!(many.validate(), Err(ReportError::Invalid("spans")));
        let mut attrs: ClientReport = serde_json::from_str(&good).unwrap();
        if let Some(s) = attrs.spans.first_mut() {
            s.attributes = (0..=MAX_ATTRIBUTES)
                .map(|i| (i.to_string(), String::new()))
                .collect();
        }
        assert_eq!(attrs.validate(), Err(ReportError::Invalid("attributes")));
        let mut page: ClientReport = serde_json::from_str(&good).unwrap();
        page.page = "x".repeat(3000);
        assert_eq!(page.validate(), Err(ReportError::Invalid("page")));
    }

    #[test]
    fn negative_offsets_saturate() {
        let anchor = UNIX_EPOCH + Duration::from_secs(10);
        assert_eq!(
            offset(anchor, -1_000_000),
            UNIX_EPOCH + Duration::from_secs(9)
        );
        assert_eq!(offset(UNIX_EPOCH, -5), UNIX_EPOCH);
    }
}
