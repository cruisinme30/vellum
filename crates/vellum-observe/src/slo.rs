// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Service level objectives and error budgets.
//!
//! The same objectives drive three things: the shipped Prometheus
//! recording and alert rules (`deploy/observability/prometheus`), the
//! Grafana SLO dashboard, and the in-process view in the admin console,
//! computed here from [`RouteMetrics`]. Alerts follow the multi-window,
//! multi-burn-rate method: page when both the 1 h and 5 min burn rates
//! exceed 14.4 (2 % of a 30-day budget in an hour).

use serde::{Deserialize, Serialize};

use crate::red::{Counts, RouteMetrics, RouteStat};

/// The burn rate that pages when sustained over both short windows.
pub const PAGE_BURN_RATE: f64 = 14.4;
/// The burn rate that opens a ticket when sustained.
pub const TICKET_BURN_RATE: f64 = 6.0;

/// What counts as a good event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Indicator {
    /// Requests that did not end in a server error.
    Availability,
    /// Requests that finished within the threshold.
    Latency {
        /// The latency threshold in milliseconds; must be a bucket bound.
        threshold_ms: u64,
    },
}

/// Which requests an objective covers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Only these methods; empty means every method.
    pub methods: Vec<String>,
    /// Only routes starting with this prefix.
    pub route_prefix: Option<String>,
}

impl Scope {
    fn covers(&self, stat: &RouteStat) -> bool {
        (self.methods.is_empty()
            || self.methods.iter().any(|m| m == &stat.method))
            && self
                .route_prefix
                .as_deref()
                .is_none_or(|p| stat.route.starts_with(p))
    }
}

/// A service level objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slo {
    /// A stable identifier, such as `availability`.
    pub id: String,
    /// What the objective promises, in plain words.
    pub description: String,
    /// The target fraction of good events, such as 0.999.
    pub objective: f64,
    /// The compliance window in days.
    pub window_days: u32,
    /// What counts as good.
    pub indicator: Indicator,
    /// Which requests count.
    pub scope: Scope,
}

impl Slo {
    /// The error budget as a fraction of events, `1 - objective`.
    #[must_use]
    pub fn budget(&self) -> f64 {
        (1.0 - self.objective).max(f64::EPSILON)
    }

    fn good(&self, counts: &Counts) -> u64 {
        match self.indicator {
            Indicator::Availability => counts.requests - counts.errors,
            Indicator::Latency { threshold_ms } => counts.within(threshold_ms),
        }
    }
}

/// The objectives from brief §12 that the server can measure itself.
#[must_use]
pub fn default_slos() -> Vec<Slo> {
    vec![
        Slo {
            id: "availability".to_owned(),
            description: "99.9 % of requests succeed each month (Org profile)."
                .to_owned(),
            objective: 0.999,
            window_days: 30,
            indicator: Indicator::Availability,
            scope: Scope::default(),
        },
        Slo {
            id: "read-latency".to_owned(),
            description:
                "95 % of reads finish within 150 ms (uncached read p95)."
                    .to_owned(),
            objective: 0.95,
            window_days: 30,
            indicator: Indicator::Latency { threshold_ms: 150 },
            scope: Scope {
                methods: vec!["GET".to_owned()],
                route_prefix: None,
            },
        },
        Slo {
            id: "write-latency".to_owned(),
            description: "99 % of writes finish within 300 ms.".to_owned(),
            objective: 0.99,
            window_days: 30,
            indicator: Indicator::Latency { threshold_ms: 300 },
            scope: Scope {
                methods: ["POST", "PUT", "PATCH", "DELETE"]
                    .map(str::to_owned)
                    .to_vec(),
                route_prefix: None,
            },
        },
    ]
}

/// How urgent an alert is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Wake someone up.
    Page,
    /// Handle during working hours.
    Ticket,
}

/// An objective's state over the in-process window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SloStatus {
    /// The objective.
    pub slo: Slo,
    /// Good events in the last hour.
    pub good: u64,
    /// All events in the last hour.
    pub total: u64,
    /// The measured fraction of good events; `None` with no traffic.
    pub sli: Option<f64>,
    /// Burn rate over the last 5 minutes (1.0 spends the budget exactly
    /// over the window).
    pub burn_rate_5m: f64,
    /// Burn rate over the last hour.
    pub burn_rate_1h: f64,
    /// Share of the error budget left if the last hour were the whole
    /// window, from 1 (untouched) down to negative (overspent).
    pub budget_remaining: f64,
    /// The alert that would fire now, if any.
    pub alert: Option<Severity>,
}

#[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
fn burn(slo: &Slo, good: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let bad_ratio = (total - good.min(total)) as f64 / total as f64;
    bad_ratio / slo.budget()
}

fn counts_for(slo: &Slo, stats: &[RouteStat]) -> (u64, u64) {
    stats.iter().filter(|s| slo.scope.covers(s)).fold(
        (0, 0),
        |(good, total), s| {
            (good + slo.good(&s.counts), total + s.counts.requests)
        },
    )
}

/// Evaluates `slo` against stats for the last 5 minutes and the last hour.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "counts are far below 2^52")]
pub fn evaluate_stats(
    slo: &Slo,
    short: &[RouteStat],
    long: &[RouteStat],
) -> SloStatus {
    let (good5, total5) = counts_for(slo, short);
    let (good, total) = counts_for(slo, long);
    let fast = burn(slo, good5, total5);
    let slow = burn(slo, good, total);
    let alert = if slow > PAGE_BURN_RATE && fast > PAGE_BURN_RATE {
        Some(Severity::Page)
    } else if slow > TICKET_BURN_RATE && fast > TICKET_BURN_RATE {
        Some(Severity::Ticket)
    } else {
        None
    };
    SloStatus {
        slo: slo.clone(),
        good,
        total,
        sli: (total > 0).then(|| good as f64 / total as f64),
        burn_rate_5m: fast,
        burn_rate_1h: slow,
        budget_remaining: 1.0 - slow,
        alert,
    }
}

/// Evaluates every objective against live metrics.
#[must_use]
pub fn evaluate(slos: &[Slo], metrics: &RouteMetrics) -> Vec<SloStatus> {
    let short = metrics.snapshot(5);
    let long = metrics.snapshot(60);
    slos.iter()
        .map(|slo| evaluate_stats(slo, &short, &long))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn healthy_traffic_has_budget() {
        let m = RouteMetrics::new();
        for _ in 0..1000 {
            m.record("GET", "/api/pages/{id}", 200, Duration::from_millis(20));
        }
        let statuses = evaluate(&default_slos(), &m);
        let availability = statuses.first().unwrap();
        assert_eq!(availability.total, 1000);
        assert_eq!(availability.sli, Some(1.0));
        assert!(availability.burn_rate_1h.abs() < f64::EPSILON);
        assert!((availability.budget_remaining - 1.0).abs() < f64::EPSILON);
        assert_eq!(availability.alert, None);
        // Writes saw no traffic.
        assert_eq!(statuses.get(2).unwrap().sli, None);
    }

    #[test]
    fn outages_page_and_slow_reads_ticket() {
        let m = RouteMetrics::new();
        for i in 0..100 {
            let status = if i < 5 { 503 } else { 200 };
            m.record("GET", "/api/a", status, Duration::from_millis(400));
        }
        let statuses = evaluate(&default_slos(), &m);
        let availability = statuses.first().unwrap();
        // 5 % errors against a 0.1 % budget is a burn rate of 50.
        assert!((availability.burn_rate_1h - 50.0).abs() < 1e-6);
        assert_eq!(availability.alert, Some(Severity::Page));
        assert!(availability.budget_remaining < 0.0);
        let reads = statuses.get(1).unwrap();
        assert_eq!(reads.good, 0);
        assert!((reads.burn_rate_1h - 20.0).abs() < 1e-6);
        assert_eq!(reads.alert, Some(Severity::Page));
    }

    #[test]
    fn ticket_threshold_and_scope() {
        let slo = Slo {
            id: "t".into(),
            description: String::new(),
            objective: 0.99,
            window_days: 30,
            indicator: Indicator::Availability,
            scope: Scope {
                methods: Vec::new(),
                route_prefix: Some("/api".into()),
            },
        };
        let m = RouteMetrics::new();
        for i in 0..100 {
            m.record(
                "GET",
                "/api/x",
                if i < 8 { 500 } else { 200 },
                Duration::from_millis(1),
            );
            m.record("GET", "/static/x", 500, Duration::from_millis(1));
        }
        let status = evaluate(std::slice::from_ref(&slo), &m).pop().unwrap();
        assert_eq!(status.total, 100);
        assert_eq!(status.alert, Some(Severity::Ticket));
        let json = serde_json::to_string(&slo.indicator).unwrap();
        assert_eq!(json, r#"{"kind":"availability"}"#);
    }
}
