// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The bounded ring buffer of recent traces behind Admin → Requests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use serde::{Deserialize, Serialize};

use crate::trace_id::TraceId;

/// The default number of traces kept (brief §12: "ring buffer of 10k traces").
pub const DEFAULT_CAPACITY: usize = 10_000;

/// The default memory budget for the buffer, in bytes.
///
/// The count cap alone cannot bound memory because a trace can hold many
/// spans; the budget keeps a full buffer well inside the single binary's
/// footprint target.
pub const DEFAULT_BYTE_BUDGET: usize = 32 * 1024 * 1024;

/// The most summaries one query returns.
pub const MAX_QUERY_LIMIT: usize = 500;

/// An event logged inside a span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanEvent {
    /// Microseconds from the start of the trace.
    pub offset_us: u64,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// The event's message.
    pub message: String,
}

/// One finished server span inside a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedSpan {
    /// Position in the trace; the root is 0.
    pub index: u32,
    /// The parent's index; `None` for the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// The span name, such as `db.query`.
    pub name: String,
    /// The `tracing` target (normally the module path).
    pub target: String,
    /// Microseconds from the start of the trace.
    pub start_us: u64,
    /// How long the span was open, in microseconds.
    pub duration_us: u64,
    /// Recorded fields, in recording order.
    pub fields: Vec<(String, String)>,
    /// Events logged inside the span.
    pub events: Vec<SpanEvent>,
    /// Whether the span failed.
    pub error: bool,
}

impl RecordedSpan {
    /// The value of a recorded field.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A span measured in the browser and joined to the server trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientSpan {
    /// What was measured, such as `navigation`, `lcp` or `fetch /api/x`.
    pub name: String,
    /// Microseconds relative to the server trace start; negative when the
    /// browser started before the server saw the request.
    pub start_us: i64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// Extra attributes, already validated.
    pub attributes: Vec<(String, String)>,
}

/// A complete trace: one request, job or command, with every span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    /// The W3C trace id, shared with OTLP exports and the browser.
    pub trace_id: TraceId,
    /// When the root span started, in Unix milliseconds.
    pub started_at_ms: u64,
    /// The root span's duration in microseconds.
    pub duration_us: u64,
    /// Server spans; index 0 is the root.
    pub spans: Vec<RecordedSpan>,
    /// Browser spans joined to this trace.
    pub client: Vec<ClientSpan>,
    /// Spans not kept because the per-trace cap was reached.
    pub dropped_spans: u32,
}

impl Trace {
    /// The root span.
    #[must_use]
    pub fn root(&self) -> Option<&RecordedSpan> {
        self.spans.iter().find(|s| s.parent.is_none())
    }

    fn root_field(&self, name: &str) -> Option<&str> {
        self.root().and_then(|r| r.field(name))
    }

    /// The HTTP status code, when the root is a request.
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        self.root_field("http.response.status_code")
            .and_then(|s| s.parse().ok())
    }

    /// Whether any span failed or the response was a server error.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.status().is_some_and(|s| s >= 500) || self.spans.iter().any(|s| s.error)
    }

    /// The summary row shown in the request list.
    #[must_use]
    pub fn summary(&self, seq: u64) -> TraceSummary {
        TraceSummary {
            seq,
            trace_id: self.trace_id,
            started_at_ms: self.started_at_ms,
            duration_us: self.duration_us,
            name: self.root().map(|r| r.name.clone()).unwrap_or_default(),
            route: self.root_field("http.route").map(str::to_owned),
            method: self.root_field("http.request.method").map(str::to_owned),
            status: self.status(),
            request_id: self.root_field("request_id").map(str::to_owned),
            error: self.is_error(),
            span_count: self.spans.len(),
            has_client: !self.client.is_empty(),
        }
    }

    /// A rough count of the heap bytes this trace holds.
    #[must_use]
    pub fn approx_bytes(&self) -> usize {
        let pairs =
            |p: &[(String, String)]| p.iter().map(|(k, v)| k.len() + v.len() + 48).sum::<usize>();
        let spans: usize = self
            .spans
            .iter()
            .map(|s| {
                std::mem::size_of::<RecordedSpan>()
                    + s.name.len()
                    + s.target.len()
                    + pairs(&s.fields)
                    + s.events
                        .iter()
                        .map(|e| std::mem::size_of::<SpanEvent>() + e.level.len() + e.message.len())
                        .sum::<usize>()
            })
            .sum();
        let client: usize = self
            .client
            .iter()
            .map(|c| std::mem::size_of::<ClientSpan>() + c.name.len() + pairs(&c.attributes))
            .sum();
        std::mem::size_of::<Self>() + spans + client
    }
}

/// One row in the request list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceSummary {
    /// Monotonic position in the buffer; use as a paging cursor.
    pub seq: u64,
    /// The trace id.
    pub trace_id: TraceId,
    /// When the trace started, in Unix milliseconds.
    pub started_at_ms: u64,
    /// Duration in microseconds.
    pub duration_us: u64,
    /// The root span name.
    pub name: String,
    /// The route template, such as `/api/pages/{id}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// The HTTP method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// The HTTP status code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The request id shown in error dialogs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Whether the trace failed.
    pub error: bool,
    /// Number of server spans.
    pub span_count: usize,
    /// Whether browser spans were joined.
    pub has_client: bool,
}

/// Filters for [`TraceStore::query`]. Every filter is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceQuery {
    /// Only this route template.
    pub route: Option<String>,
    /// Only traces at least this long, in milliseconds.
    pub min_duration_ms: Option<u64>,
    /// Only failed traces.
    pub errors_only: bool,
    /// Only traces whose request id or trace id equals this value.
    pub id: Option<String>,
    /// Only traces older than this cursor (`seq`).
    pub before: Option<u64>,
    /// At most this many rows (default 100, capped at [`MAX_QUERY_LIMIT`]).
    pub limit: Option<usize>,
}

/// Buffer counters for the admin console and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreStats {
    /// Traces currently held.
    pub traces: usize,
    /// Approximate bytes currently held.
    pub bytes: usize,
    /// Maximum traces held.
    pub capacity: usize,
    /// Memory budget in bytes.
    pub byte_budget: usize,
    /// Traces ever recorded.
    pub recorded: u64,
    /// Traces evicted to stay inside the limits.
    pub evicted: u64,
}

#[derive(Debug)]
struct Entry {
    seq: u64,
    bytes: usize,
    trace: Arc<Trace>,
}

#[derive(Debug, Default)]
struct Inner {
    entries: VecDeque<Entry>,
    bytes: usize,
    recorded: u64,
    evicted: u64,
}

/// A bounded, thread-safe ring buffer of finished traces.
///
/// The oldest traces are evicted first once either the count capacity or
/// the byte budget is exceeded. Every operation holds the lock briefly and
/// never performs I/O.
#[derive(Debug)]
pub struct TraceStore {
    inner: Mutex<Inner>,
    capacity: usize,
    byte_budget: usize,
    evictions: Counter<u64>,
}

impl Default for TraceStore {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_BYTE_BUDGET)
    }
}

impl TraceStore {
    /// A buffer holding at most `capacity` traces and about `byte_budget`
    /// bytes. Both are raised to at least 1.
    #[must_use]
    pub fn new(capacity: usize, byte_budget: usize) -> Self {
        let evictions = opentelemetry::global::meter("vellum-observe")
            .u64_counter("vellum.observe.traces.evicted")
            .with_description("Traces evicted from the Admin → Requests ring buffer")
            .build();
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            byte_budget: byte_budget.max(1),
            evictions,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave the buffer in a state
        // worse than "missing one trace", so poisoning is ignored.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds a finished trace, evicting the oldest ones as needed. Returns
    /// its sequence number.
    pub fn push(&self, trace: Trace) -> u64 {
        let bytes = trace.approx_bytes();
        let mut inner = self.lock();
        inner.recorded += 1;
        let seq = inner.recorded;
        inner.bytes += bytes;
        inner.entries.push_back(Entry {
            seq,
            bytes,
            trace: Arc::new(trace),
        });
        let mut evicted = 0_u64;
        while inner.entries.len() > self.capacity
            || (inner.bytes > self.byte_budget && inner.entries.len() > 1)
        {
            if let Some(old) = inner.entries.pop_front() {
                inner.bytes -= old.bytes;
                evicted += 1;
            }
        }
        inner.evicted += evicted;
        drop(inner);
        if evicted > 0 {
            self.evictions
                .add(evicted, &[KeyValue::new("reason", "limit")]);
        }
        seq
    }

    /// The trace with this id, if it is still held.
    #[must_use]
    pub fn get(&self, id: TraceId) -> Option<Arc<Trace>> {
        self.lock()
            .entries
            .iter()
            .rev()
            .find(|e| e.trace.trace_id == id)
            .map(|e| Arc::clone(&e.trace))
    }

    /// Summaries of matching traces, newest first.
    #[must_use]
    pub fn query(&self, query: &TraceQuery) -> Vec<TraceSummary> {
        let limit = query.limit.unwrap_or(100).clamp(1, MAX_QUERY_LIMIT);
        let min_us = query.min_duration_ms.map(|ms| ms.saturating_mul(1000));
        self.lock()
            .entries
            .iter()
            .rev()
            .filter(|e| query.before.is_none_or(|b| e.seq < b))
            .map(|e| e.trace.summary(e.seq))
            .filter(|s| {
                query
                    .route
                    .as_deref()
                    .is_none_or(|r| s.route.as_deref() == Some(r))
            })
            .filter(|s| min_us.is_none_or(|m| s.duration_us >= m))
            .filter(|s| !query.errors_only || s.error)
            .filter(|s| {
                query.id.as_deref().is_none_or(|id| {
                    s.request_id.as_deref() == Some(id) || s.trace_id.to_string() == id
                })
            })
            .take(limit)
            .collect()
    }

    /// Every trace that started at or after `since_ms` (Unix milliseconds),
    /// oldest first.
    #[must_use]
    pub fn since(&self, since_ms: u64) -> Vec<Arc<Trace>> {
        self.lock()
            .entries
            .iter()
            .filter(|e| e.trace.started_at_ms >= since_ms)
            .map(|e| Arc::clone(&e.trace))
            .collect()
    }

    /// Joins browser spans to a held trace. Returns `false` when the trace
    /// is no longer (or not yet) in the buffer.
    pub fn attach_client(&self, id: TraceId, spans: Vec<ClientSpan>) -> bool {
        let mut inner = self.lock();
        let Some(index) = inner.entries.iter().rposition(|e| e.trace.trace_id == id) else {
            return false;
        };
        let Some(entry) = inner.entries.get_mut(index) else {
            return false;
        };
        let mut trace = (*entry.trace).clone();
        trace.client.extend(spans);
        let bytes = trace.approx_bytes();
        let old = std::mem::replace(&mut entry.bytes, bytes);
        entry.trace = Arc::new(trace);
        inner.bytes = inner.bytes - old + bytes;
        true
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> StoreStats {
        let inner = self.lock();
        StoreStats {
            traces: inner.entries.len(),
            bytes: inner.bytes,
            capacity: self.capacity,
            byte_budget: self.byte_budget,
            recorded: inner.recorded,
            evicted: inner.evicted,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn span(index: u32, parent: Option<u32>, name: &str, start: u64, dur: u64) -> RecordedSpan {
        RecordedSpan {
            index,
            parent,
            name: name.to_owned(),
            target: "vellum_test".to_owned(),
            start_us: start,
            duration_us: dur,
            fields: Vec::new(),
            events: Vec::new(),
            error: false,
        }
    }

    pub fn trace(route: &str, status: u16, dur: u64, started: u64) -> Trace {
        let mut root = span(0, None, "http.request", 0, dur);
        root.fields = vec![
            ("http.route".into(), route.into()),
            ("http.request.method".into(), "GET".into()),
            ("http.response.status_code".into(), status.to_string()),
            ("request_id".into(), format!("req-{started}")),
        ];
        Trace {
            trace_id: TraceId::random(),
            started_at_ms: started,
            duration_us: dur,
            spans: vec![root, span(1, Some(0), "db.query", 10, dur / 2)],
            client: Vec::new(),
            dropped_spans: 0,
        }
    }

    #[test]
    fn evicts_by_count() {
        let store = TraceStore::new(3, usize::MAX);
        for i in 0..5 {
            let _ = store.push(trace("/a", 200, 1000, i));
        }
        let stats = store.stats();
        assert_eq!(stats.traces, 3);
        assert_eq!(stats.recorded, 5);
        assert_eq!(stats.evicted, 2);
        let rows = store.query(&TraceQuery::default());
        assert_eq!(
            rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![5, 4, 3]
        );
    }

    #[test]
    fn evicts_by_bytes_but_keeps_the_newest() {
        let one = trace("/a", 200, 1, 0).approx_bytes();
        let store = TraceStore::new(100, one * 2 + one / 2);
        for i in 0..4 {
            let _ = store.push(trace("/a", 200, 1, i));
        }
        assert_eq!(store.stats().traces, 2);
        let tiny = TraceStore::new(100, 1);
        let _ = tiny.push(trace("/a", 200, 1, 0));
        assert_eq!(tiny.stats().traces, 1);
    }

    #[test]
    fn filters_and_pages() {
        let store = TraceStore::default();
        let _ = store.push(trace("/a", 200, 5_000, 1));
        let _ = store.push(trace("/b", 503, 50_000, 2));
        let _ = store.push(trace("/a", 200, 90_000, 3));
        let route = TraceQuery {
            route: Some("/a".into()),
            ..TraceQuery::default()
        };
        assert_eq!(store.query(&route).len(), 2);
        let slow = TraceQuery {
            min_duration_ms: Some(40),
            ..TraceQuery::default()
        };
        assert_eq!(store.query(&slow).len(), 2);
        let errors = TraceQuery {
            errors_only: true,
            ..TraceQuery::default()
        };
        let rows = store.query(&errors);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.first().and_then(|r| r.status), Some(503));
        let paged = TraceQuery {
            before: Some(3),
            limit: Some(1),
            ..TraceQuery::default()
        };
        assert_eq!(store.query(&paged).first().map(|r| r.seq), Some(2));
        let by_req = TraceQuery {
            id: Some("req-1".into()),
            ..TraceQuery::default()
        };
        assert_eq!(store.query(&by_req).len(), 1);
    }

    #[test]
    fn get_and_attach_client() {
        let store = TraceStore::default();
        let t = trace("/a", 200, 1000, 10);
        let id = t.trace_id;
        let _ = store.push(t);
        let before = store.stats().bytes;
        assert!(store.attach_client(
            id,
            vec![ClientSpan {
                name: "lcp".into(),
                start_us: -500,
                duration_us: 2000,
                attributes: vec![("page".into(), "/".into())],
            }]
        ));
        let got = store.get(id).unwrap();
        assert_eq!(got.client.len(), 1);
        assert!(got.summary(1).has_client);
        assert!(store.stats().bytes > before);
        assert!(!store.attach_client(TraceId::random(), Vec::new()));
        assert!(store.get(TraceId::random()).is_none());
    }

    #[test]
    fn since_returns_recent_oldest_first() {
        let store = TraceStore::default();
        for i in 0..5 {
            let _ = store.push(trace("/a", 200, 1, i * 10));
        }
        let recent = store.since(20);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent.first().map(|t| t.started_at_ms), Some(20));
    }

    #[test]
    fn span_errors_mark_the_trace() {
        let mut t = trace("/a", 200, 1, 0);
        assert!(!t.is_error());
        if let Some(s) = t.spans.get_mut(1) {
            s.error = true;
        }
        assert!(t.is_error());
    }
}
