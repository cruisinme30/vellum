// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A `tracing` layer that assembles finished span trees into [`Trace`]s
//! and keeps them in a [`TraceStore`].
//!
//! The layer sits next to the OpenTelemetry layer from
//! `vellum_kernel::telemetry`, so the single binary shows every request in
//! Admin → Requests without any collector. Root spans whose target does not
//! start with one of the configured prefixes are ignored, which keeps
//! library-internal spans (connection pools, HTTP clients) from filling the
//! buffer on their own.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::store::{RecordedSpan, SpanEvent, Trace, TraceStore};
use crate::trace_id::TraceId;

/// The field a root span uses to carry its W3C trace id.
pub const TRACE_ID_FIELD: &str = "trace_id";

/// Limits applied while recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderConfig {
    /// Root spans are recorded only when their target starts with one of
    /// these prefixes.
    pub root_targets: Vec<String>,
    /// Most spans kept per trace; later ones are counted as dropped.
    pub max_spans_per_trace: u32,
    /// Most fields kept per span.
    pub max_fields: usize,
    /// Most events kept per span.
    pub max_events: usize,
    /// Longest field value or message kept, in bytes; longer ones are cut.
    pub max_value_len: usize,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            root_targets: vec!["vellum".to_owned()],
            max_spans_per_trace: 512,
            max_fields: 32,
            max_events: 16,
            max_value_len: 512,
        }
    }
}

/// Shared by every span of one trace while it is open.
#[derive(Debug)]
struct TraceState {
    trace_id: Mutex<TraceId>,
    start: Instant,
    started_at_ms: u64,
    next_index: AtomicU32,
    dropped: AtomicU32,
    finished: Mutex<Vec<RecordedSpan>>,
}

/// Kept in each recorded span's extensions.
#[derive(Debug)]
struct SpanState {
    trace: Arc<TraceState>,
    index: u32,
    parent: Option<u32>,
    start: Instant,
    fields: Vec<(String, String)>,
    events: Vec<SpanEvent>,
    error: bool,
}

/// The recording layer. Install it with
/// `tracing_subscriber::layer::SubscriberExt::with`.
#[derive(Debug, Clone)]
pub struct RequestRecorder {
    store: Arc<TraceStore>,
    config: Arc<RecorderConfig>,
}

impl RequestRecorder {
    /// A recorder writing into `store`.
    #[must_use]
    pub fn new(store: Arc<TraceStore>, config: RecorderConfig) -> Self {
        Self {
            store,
            config: Arc::new(config),
        }
    }

    /// The store traces are written to.
    #[must_use]
    pub const fn store(&self) -> &Arc<TraceStore> {
        &self.store
    }

    fn records_root(&self, target: &str) -> bool {
        self.config
            .root_targets
            .iter()
            .any(|prefix| target.starts_with(prefix.as_str()))
    }
}

fn unix_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn micros(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

fn truncate(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// Collects fields into `(name, value)` pairs.
struct FieldVisitor<'a> {
    fields: &'a mut Vec<(String, String)>,
    message: Option<&'a mut String>,
    config: &'a RecorderConfig,
}

impl FieldVisitor<'_> {
    fn put(&mut self, field: &Field, value: String) {
        if field.name() == "message"
            && let Some(message) = self.message.as_deref_mut()
        {
            *message = truncate(value, self.config.max_value_len);
            return;
        }
        let value = truncate(value, self.config.max_value_len);
        if let Some(slot) = self.fields.iter_mut().find(|(k, _)| k == field.name()) {
            slot.1 = value;
        } else if self.fields.len() < self.config.max_fields {
            self.fields.push((field.name().to_owned(), value));
        }
    }
}

impl Visit for FieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_owned());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{value:?}");
        self.put(field, s);
    }
}

fn is_error_field(fields: &[(String, String)]) -> bool {
    fields.iter().any(|(k, v)| {
        (k == "error" && v != "false" && !v.is_empty())
            || (k == "otel.status_code" && v.eq_ignore_ascii_case("error"))
    })
}

fn trace_id_from(fields: &[(String, String)]) -> Option<TraceId> {
    fields
        .iter()
        .find(|(k, _)| k == TRACE_ID_FIELD)
        .and_then(|(_, v)| v.parse().ok())
}

impl<S> Layer<S> for RequestRecorder
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let now = Instant::now();
        let mut fields = Vec::new();
        attrs.record(&mut FieldVisitor {
            fields: &mut fields,
            message: None,
            config: &self.config,
        });
        let (trace, index, parent) = match span.parent() {
            None => {
                if !self.records_root(span.metadata().target()) {
                    return;
                }
                let trace = Arc::new(TraceState {
                    trace_id: Mutex::new(trace_id_from(&fields).unwrap_or_else(TraceId::random)),
                    start: now,
                    started_at_ms: unix_ms(SystemTime::now()),
                    next_index: AtomicU32::new(1),
                    dropped: AtomicU32::new(0),
                    finished: Mutex::new(Vec::new()),
                });
                (trace, 0, None)
            }
            Some(parent) => {
                let extensions = parent.extensions();
                let Some(parent_state) = extensions.get::<SpanState>() else {
                    return;
                };
                let trace = Arc::clone(&parent_state.trace);
                let parent_index = parent_state.index;
                drop(extensions);
                let index = trace.next_index.fetch_add(1, Ordering::Relaxed);
                if index >= self.config.max_spans_per_trace {
                    let _ = trace.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                (trace, index, Some(parent_index))
            }
        };
        let error = is_error_field(&fields);
        span.extensions_mut().insert(SpanState {
            trace,
            index,
            parent,
            start: now,
            fields,
            events: Vec::new(),
            error,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut extensions = span.extensions_mut();
        let Some(state) = extensions.get_mut::<SpanState>() else {
            return;
        };
        values.record(&mut FieldVisitor {
            fields: &mut state.fields,
            message: None,
            config: &self.config,
        });
        state.error = state.error || is_error_field(&state.fields);
        if state.parent.is_none()
            && let Some(trace_id) = trace_id_from(&state.fields)
        {
            *state
                .trace
                .trace_id
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = trace_id;
        }
        drop(extensions);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.event_span(event) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        let Some(state) = extensions.get_mut::<SpanState>() else {
            return;
        };
        let level = *event.metadata().level();
        if level == Level::ERROR {
            state.error = true;
        }
        if state.events.len() >= self.config.max_events {
            return;
        }
        let mut message = String::new();
        let mut fields = Vec::new();
        event.record(&mut FieldVisitor {
            fields: &mut fields,
            message: Some(&mut message),
            config: &self.config,
        });
        for (k, v) in fields {
            if !message.is_empty() {
                message.push(' ');
            }
            let _ = write!(message, "{k}={v}");
        }
        state.events.push(SpanEvent {
            offset_us: micros(state.trace.start.elapsed()),
            level: level.to_string(),
            message: truncate(message, self.config.max_value_len),
        });
        drop(extensions);
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(state) = span.extensions_mut().remove::<SpanState>() else {
            return;
        };
        let recorded = RecordedSpan {
            index: state.index,
            parent: state.parent,
            name: span.metadata().name().to_owned(),
            target: span.metadata().target().to_owned(),
            start_us: micros(state.start.saturating_duration_since(state.trace.start)),
            duration_us: micros(state.start.elapsed()),
            fields: state.fields,
            events: state.events,
            error: state.error,
        };
        let trace = state.trace;
        if recorded.parent.is_some() {
            trace
                .finished
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(recorded);
            return;
        }
        let mut spans = std::mem::take(
            &mut *trace
                .finished
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let duration_us = recorded.duration_us;
        spans.push(recorded);
        spans.sort_by_key(|s| (s.start_us, s.index));
        let trace_id = *trace
            .trace_id
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let _ = self.store.push(Trace {
            trace_id,
            started_at_ms: trace.started_at_ms,
            duration_us,
            spans,
            client: Vec::new(),
            dropped_spans: trace.dropped.load(Ordering::Relaxed),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TraceQuery;
    use tracing_subscriber::layer::SubscriberExt as _;

    fn with_recorder(config: RecorderConfig, f: impl FnOnce()) -> Arc<TraceStore> {
        let store = Arc::new(TraceStore::default());
        let recorder = RequestRecorder::new(Arc::clone(&store), config);
        assert!(Arc::ptr_eq(recorder.store(), &store));
        let subscriber = tracing_subscriber::registry().with(recorder);
        let _lock = crate::TRACING_TEST_LOCK.blocking_lock();
        tracing::subscriber::with_default(subscriber, f);
        store
    }

    #[test]
    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "route templates use braces"
    )]
    fn records_a_span_tree() {
        let id = TraceId::random();
        let store = with_recorder(RecorderConfig::default(), || {
            let root = tracing::info_span!(
                target: "vellum_api",
                "http.request",
                trace_id = %id,
                http.route = "/api/pages/{id}",
                http.response.status_code = tracing::field::Empty,
            );
            {
                let _r = root.enter();
                let db = tracing::info_span!("db.query", db.statement = "SELECT 1", rows = 1_u64);
                let _d = db.enter();
                tracing::info!(cache = "miss", "looked up");
                let inner = tracing::info_span!("authz.check");
                drop(inner.enter());
            }
            let _ = root.record("http.response.status_code", 200_u16);
        });
        let rows = store.query(&TraceQuery::default());
        assert_eq!(rows.len(), 1);
        let row = rows.first().unwrap();
        assert_eq!(row.trace_id, id);
        assert_eq!(row.route.as_deref(), Some("/api/pages/{id}"));
        assert_eq!(row.status, Some(200));
        assert!(!row.error);
        let trace = store.get(id).unwrap();
        assert_eq!(trace.spans.len(), 3);
        let db = trace.spans.iter().find(|s| s.name == "db.query").unwrap();
        assert_eq!(db.parent, Some(0));
        assert_eq!(db.field("db.statement"), Some("SELECT 1"));
        assert_eq!(db.events.len(), 1);
        assert!(
            db.events
                .first()
                .unwrap()
                .message
                .contains("looked up cache=miss")
        );
        let authz = trace
            .spans
            .iter()
            .find(|s| s.name == "authz.check")
            .unwrap();
        assert_eq!(authz.parent, Some(db.index));
    }

    #[test]
    fn ignores_foreign_roots_and_marks_errors() {
        let store = with_recorder(RecorderConfig::default(), || {
            let foreign = tracing::info_span!(target: "hyper::pool", "checkout");
            drop(foreign.enter());
            let root = tracing::info_span!(target: "vellum_jobs", "job.render");
            let _r = root.enter();
            tracing::error!("render failed");
        });
        let rows = store.query(&TraceQuery::default());
        assert_eq!(rows.len(), 1);
        assert!(rows.first().unwrap().error);
        assert_eq!(rows.first().unwrap().name, "job.render");
    }

    #[test]
    fn caps_spans_fields_events_and_values() {
        let config = RecorderConfig {
            max_spans_per_trace: 3,
            max_fields: 2,
            max_events: 1,
            max_value_len: 5,
            ..RecorderConfig::default()
        };
        let store = with_recorder(config, || {
            let root = tracing::info_span!(target: "vellum", "root", a = "abcdefgh", b = 1, c = 2);
            let _r = root.enter();
            tracing::info!("one");
            tracing::info!("two");
            for _ in 0..5 {
                let child = tracing::info_span!("child", otel.status_code = "ERROR");
                drop(child.enter());
            }
        });
        let rows = store.query(&TraceQuery::default());
        let trace = store.get(rows.first().unwrap().trace_id).unwrap();
        assert_eq!(trace.spans.len(), 3);
        assert_eq!(trace.dropped_spans, 3);
        let root = trace.root().unwrap();
        assert_eq!(root.fields.len(), 2);
        assert_eq!(root.field("a"), Some("abcde…"));
        assert_eq!(root.events.len(), 1);
        assert!(trace.is_error());
    }

    #[test]
    fn late_trace_id_and_error_records() {
        let id = TraceId::random();
        let store = with_recorder(RecorderConfig::default(), || {
            let root = tracing::info_span!(
                target: "vellum",
                "root",
                trace_id = tracing::field::Empty,
                error = tracing::field::Empty
            );
            let _ = root.record(TRACE_ID_FIELD, id.to_string());
            let _ = root.record("error", true);
            let err = std::io::Error::other("disk gone");
            let child = tracing::info_span!(parent: &root, "io", cause = tracing::field::Empty);
            let _ = child.record("cause", &err as &(dyn std::error::Error + 'static));
            drop(child);
        });
        let trace = store.get(id).unwrap();
        assert!(trace.root().unwrap().error);
        assert_eq!(
            trace
                .spans
                .iter()
                .find(|s| s.name == "io")
                .unwrap()
                .field("cause"),
            Some("disk gone")
        );
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo".to_owned(), 2), "h…");
        assert_eq!(truncate("hi".to_owned(), 5), "hi");
    }
}
