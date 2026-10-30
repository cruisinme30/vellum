// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RED (rate, errors, duration) and saturation metrics per route.
//!
//! Every request is recorded twice: into OpenTelemetry instruments that
//! follow the HTTP semantic conventions (exported over OTLP and charted by
//! the shipped Grafana dashboards), and into a small in-process window that
//! the admin console reads when no collector is running. Labels are the
//! route *template*, method and status code only, so cardinality stays
//! bounded whatever the URLs contain.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Histogram, UpDownCounter};
use serde::{Deserialize, Serialize};

/// Latency bucket upper bounds, in milliseconds. They include every
/// latency target in brief §12 (50, 120, 150, 200, 300 ms and 3 s) so SLOs
/// can count good events exactly.
pub const BUCKETS_MS: [u64; 18] = [
    5, 10, 25, 50, 75, 100, 120, 150, 200, 300, 500, 750, 1_000, 2_000, 3_000,
    5_000, 10_000, 30_000,
];

const SLOTS: usize = 60;

/// How many one-minute slots the in-process window keeps.
pub const WINDOW_MINUTES: u64 = SLOTS as u64;

/// The route label for requests that matched no route.
pub const UNMATCHED_ROUTE: &str = "unmatched";

/// Request counts and a latency histogram for a span of time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Requests finished.
    pub requests: u64,
    /// Requests that ended in a server error (5xx).
    pub errors: u64,
    /// Requests per bucket of [`BUCKETS_MS`], plus a final overflow bucket.
    pub buckets: Vec<u64>,
}

impl Counts {
    fn empty() -> Self {
        Self {
            requests: 0,
            errors: 0,
            buckets: vec![0; BUCKETS_MS.len() + 1],
        }
    }

    fn add(&mut self, other: &Self) {
        self.requests += other.requests;
        self.errors += other.errors;
        for (a, b) in self.buckets.iter_mut().zip(&other.buckets) {
            *a += *b;
        }
    }

    /// Requests that finished within `threshold_ms`. The threshold is
    /// rounded down to the nearest bucket bound.
    #[must_use]
    pub fn within(&self, threshold_ms: u64) -> u64 {
        let n = BUCKETS_MS
            .iter()
            .take_while(|b| **b <= threshold_ms)
            .count();
        self.buckets.iter().take(n).sum()
    }

    /// The latency at quantile `q` (0–1) in milliseconds, interpolated
    /// linearly inside its bucket; `None` with no requests.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "request counts and millisecond bounds are far below 2^52"
    )]
    pub fn quantile_ms(&self, q: f64) -> Option<f64> {
        let total: u64 = self.buckets.iter().sum();
        if total == 0 {
            return None;
        }
        let rank = (q.clamp(0.0, 1.0) * total as f64).ceil().max(1.0) as u64;
        let mut seen = 0_u64;
        let mut lower = 0_u64;
        for (i, count) in self.buckets.iter().enumerate() {
            let upper = BUCKETS_MS.get(i).copied();
            if seen + count >= rank {
                let Some(upper) = upper else {
                    // Overflow bucket: report its lower bound.
                    return Some(lower as f64);
                };
                let into = (rank - seen) as f64 / (*count).max(1) as f64;
                return Some(
                    ((upper - lower) as f64).mul_add(into, lower as f64),
                );
            }
            seen += count;
            lower = upper.unwrap_or(lower);
        }
        Some(lower as f64)
    }
}

fn bucket_of(duration: Duration) -> usize {
    let ms = duration.as_secs_f64() * 1000.0;
    BUCKETS_MS
        .iter()
        .position(|b| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "bounds are small integers"
            )]
            let bound = *b as f64;
            ms <= bound
        })
        .unwrap_or(BUCKETS_MS.len())
}

#[derive(Debug, Clone)]
struct Slot {
    minute: u64,
    counts: Counts,
}

#[derive(Debug)]
struct RouteWindow {
    slots: Vec<Slot>,
    in_flight: i64,
}

impl RouteWindow {
    fn new() -> Self {
        Self {
            slots: vec![
                Slot {
                    minute: u64::MAX,
                    counts: Counts::empty(),
                };
                SLOTS
            ],
            in_flight: 0,
        }
    }

    fn record(&mut self, minute: u64, error: bool, bucket: usize) {
        let index = usize::try_from(minute % WINDOW_MINUTES).unwrap_or(0);
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        if slot.minute != minute {
            slot.minute = minute;
            slot.counts = Counts::empty();
        }
        slot.counts.requests += 1;
        slot.counts.errors += u64::from(error);
        if let Some(b) = slot.counts.buckets.get_mut(bucket) {
            *b += 1;
        }
    }

    fn sum(&self, now_minute: u64, minutes: u64) -> Counts {
        let oldest = now_minute.saturating_sub(minutes.saturating_sub(1));
        let mut total = Counts::empty();
        for slot in &self.slots {
            if slot.minute != u64::MAX
                && slot.minute >= oldest
                && slot.minute <= now_minute
            {
                total.add(&slot.counts);
            }
        }
        total
    }
}

/// RED and saturation figures for one route over a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteStat {
    /// The HTTP method.
    pub method: String,
    /// The route template.
    pub route: String,
    /// Requests per second over the window.
    pub rate_per_s: f64,
    /// Fraction of requests that were server errors.
    pub error_ratio: f64,
    /// Median latency in milliseconds.
    pub p50_ms: Option<f64>,
    /// 95th percentile latency in milliseconds.
    pub p95_ms: Option<f64>,
    /// 99th percentile latency in milliseconds.
    pub p99_ms: Option<f64>,
    /// Requests in progress now (saturation).
    pub in_flight: i64,
    /// The raw counts behind the figures.
    pub counts: Counts,
    /// The p95 budget in milliseconds, once [`Budgets::annotate`] has run.
    ///
    /// [`Budgets::annotate`]: crate::budget::Budgets::annotate
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_ms: Option<u64>,
    /// Whether the p95 is over budget with enough requests to tell.
    #[serde(default)]
    pub over_budget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RouteKey {
    method: String,
    route: String,
}

#[derive(Debug)]
struct Instruments {
    duration: Histogram<f64>,
    active: UpDownCounter<i64>,
}

#[derive(Debug)]
struct Inner {
    routes: HashMap<RouteKey, RouteWindow>,
}

/// Per-route RED and saturation metrics.
#[derive(Debug, Clone)]
pub struct RouteMetrics {
    inner: Arc<Mutex<Inner>>,
    instruments: Arc<Instruments>,
}

impl Default for RouteMetrics {
    fn default() -> Self {
        Self::new()
    }
}

fn now_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 60)
}

/// Decrements the in-flight gauge when the request finishes or is dropped.
#[derive(Debug)]
#[must_use = "the request counts as in flight until the guard is dropped"]
pub struct InFlight {
    metrics: RouteMetrics,
    key: RouteKey,
    attrs: [KeyValue; 2],
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.metrics.instruments.active.add(-1, &self.attrs);
        if let Some(w) = self.metrics.lock().routes.get_mut(&self.key) {
            w.in_flight -= 1;
        }
    }
}

impl RouteMetrics {
    /// Creates the instruments on the global meter provider.
    #[must_use]
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("vellum-observe");
        let boundaries = BUCKETS_MS
            .iter()
            .map(|ms| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "bounds are small integers"
                )]
                let s = *ms as f64 / 1000.0;
                s
            })
            .collect();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                routes: HashMap::new(),
            })),
            instruments: Arc::new(Instruments {
                duration: meter
                    .f64_histogram("http.server.request.duration")
                    .with_unit("s")
                    .with_description("Duration of HTTP server requests")
                    .with_boundaries(boundaries)
                    .build(),
                active: meter
                    .i64_up_down_counter("http.server.active_requests")
                    .with_unit("{request}")
                    .with_description("HTTP server requests in progress")
                    .build(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks a request as started; drop the guard when it finishes.
    pub fn start(&self, method: &str, route: &str) -> InFlight {
        let key = RouteKey {
            method: method.to_owned(),
            route: route.to_owned(),
        };
        let attrs = [
            KeyValue::new("http.request.method", method.to_owned()),
            KeyValue::new("http.route", route.to_owned()),
        ];
        self.instruments.active.add(1, &attrs);
        self.lock()
            .routes
            .entry(key.clone())
            .or_insert_with(RouteWindow::new)
            .in_flight += 1;
        InFlight {
            metrics: self.clone(),
            key,
            attrs,
        }
    }

    /// Records one finished request.
    pub fn record(
        &self,
        method: &str,
        route: &str,
        status: u16,
        duration: Duration,
    ) {
        self.record_at(now_minute(), method, route, status, duration);
    }

    /// Like [`Self::record`], filed under `minute` (minutes since the Unix
    /// epoch) instead of the current one, so tests need no clock.
    pub fn record_at(
        &self,
        minute: u64,
        method: &str,
        route: &str,
        status: u16,
        duration: Duration,
    ) {
        let error = status >= 500;
        self.instruments.duration.record(
            duration.as_secs_f64(),
            &[
                KeyValue::new("http.request.method", method.to_owned()),
                KeyValue::new("http.route", route.to_owned()),
                KeyValue::new("http.response.status_code", i64::from(status)),
            ],
        );
        self.lock()
            .routes
            .entry(RouteKey {
                method: method.to_owned(),
                route: route.to_owned(),
            })
            .or_insert_with(RouteWindow::new)
            .record(minute, error, bucket_of(duration));
    }

    /// Per-route figures over the last `minutes` minutes (1–60), busiest
    /// route first.
    #[must_use]
    pub fn snapshot(&self, minutes: u64) -> Vec<RouteStat> {
        self.snapshot_at(now_minute(), minutes)
    }

    /// Every route's counts over the last `minutes` minutes, summed.
    #[must_use]
    pub fn totals(&self, minutes: u64) -> Counts {
        let mut total = Counts::empty();
        for stat in self.snapshot(minutes) {
            total.add(&stat.counts);
        }
        total
    }

    /// Like [`Self::snapshot`], as if `now` (minutes since the Unix epoch)
    /// were the current minute.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
    pub fn snapshot_at(&self, now: u64, minutes: u64) -> Vec<RouteStat> {
        let minutes = minutes.clamp(1, WINDOW_MINUTES);
        let mut stats: Vec<RouteStat> = self
            .lock()
            .routes
            .iter()
            .map(|(key, window)| {
                let counts = window.sum(now, minutes);
                let requests = counts.requests;
                RouteStat {
                    method: key.method.clone(),
                    route: key.route.clone(),
                    rate_per_s: requests as f64 / (minutes * 60) as f64,
                    error_ratio: if requests == 0 {
                        0.0
                    } else {
                        counts.errors as f64 / requests as f64
                    },
                    p50_ms: counts.quantile_ms(0.50),
                    p95_ms: counts.quantile_ms(0.95),
                    p99_ms: counts.quantile_ms(0.99),
                    in_flight: window.in_flight,
                    counts,
                    budget_ms: None,
                    over_budget: false,
                }
            })
            .filter(|s| s.counts.requests > 0 || s.in_flight > 0)
            .collect();
        stats.sort_by(|a, b| {
            b.counts
                .requests
                .cmp(&a.counts.requests)
                .then_with(|| a.route.cmp(&b.route))
                .then_with(|| a.method.cmp(&b.method))
        });
        stats
    }

    /// Requests in progress across every route.
    #[must_use]
    pub fn in_flight(&self) -> i64 {
        self.lock().routes.values().map(|w| w.in_flight).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn buckets_include_bounds() {
        assert_eq!(bucket_of(ms(0)), 0);
        assert_eq!(bucket_of(ms(5)), 0);
        assert_eq!(bucket_of(ms(6)), 1);
        assert_eq!(bucket_of(ms(50)), 3);
        assert_eq!(bucket_of(ms(60_000)), BUCKETS_MS.len());
    }

    #[test]
    fn quantiles_interpolate() {
        let mut c = Counts::empty();
        assert_eq!(c.quantile_ms(0.5), None);
        // 100 requests, all in the 25-50 ms bucket.
        if let Some(b) = c.buckets.get_mut(3) {
            *b = 100;
        }
        let p50 = c.quantile_ms(0.5).unwrap();
        assert!((p50 - 37.5).abs() < 0.01, "{p50}");
        assert!((c.quantile_ms(1.0).unwrap() - 50.0).abs() < 0.01);
        assert_eq!(c.within(50), 100);
        assert_eq!(c.within(49), 0);
        let mut over = Counts::empty();
        if let Some(b) = over.buckets.last_mut() {
            *b = 3;
        }
        assert_eq!(over.quantile_ms(0.99), Some(30_000.0));
    }

    #[test]
    fn records_and_windows() {
        let m = RouteMetrics::new();
        for i in 0..10 {
            m.record_at(
                100,
                "GET",
                "/a",
                if i == 0 { 500 } else { 200 },
                ms(20),
            );
        }
        m.record_at(101, "POST", "/b", 201, ms(200));
        m.record_at(50, "GET", "/a", 200, ms(1));
        let stats = m.snapshot_at(101, 5);
        assert_eq!(stats.len(), 2);
        let a = stats.first().unwrap();
        assert_eq!(a.route, "/a");
        assert_eq!(a.counts.requests, 10);
        assert!((a.error_ratio - 0.1).abs() < 1e-9);
        assert!((a.rate_per_s - 10.0 / 300.0).abs() < 1e-9);
        assert!(a.p95_ms.unwrap() <= 25.0);
        // The old minute is outside a 5-minute window but inside 60 minutes
        // only while its slot has not been reused.
        assert_eq!(m.snapshot_at(101, 60).first().unwrap().counts.requests, 11);
        // Minute 160 reuses minute 100's slot and resets it.
        m.record_at(160, "GET", "/a", 200, ms(1));
        assert_eq!(m.snapshot_at(160, 1).first().unwrap().counts.requests, 1);
    }

    #[test]
    fn in_flight_tracks_guards() {
        let m = RouteMetrics::default();
        let g1 = m.start("GET", "/a");
        let g2 = m.start("GET", "/a");
        assert_eq!(m.in_flight(), 2);
        let stats = m.snapshot(1);
        assert_eq!(stats.first().unwrap().in_flight, 2);
        drop(g1);
        drop(g2);
        assert_eq!(m.in_flight(), 0);
        assert!(m.snapshot(1).is_empty());
        m.record("GET", "/a", 200, ms(3));
        assert_eq!(m.totals(1).requests, 1);
    }
}
