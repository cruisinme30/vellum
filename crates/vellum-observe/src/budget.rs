// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Latency budgets per route (R-OB-4).
//!
//! Every route has a p95 budget. Reads default to 150 ms and writes to
//! 300 ms, the same targets as the read and write latency SLOs; a rule can
//! give one route, or a group of routes, its own budget. The console marks
//! routes over budget, Prometheus alerts when one stays over budget, and
//! `vellum budgets` turns the same check into a CI gate after a load test.

use serde::{Deserialize, Serialize};

use crate::red::RouteStat;

/// The default p95 budget for reads (`GET`, `HEAD`), in milliseconds.
pub const DEFAULT_READ_MS: u64 = 150;
/// The default p95 budget for writes, in milliseconds.
pub const DEFAULT_WRITE_MS: u64 = 300;
/// Fewer requests than this in the window are too few to judge a p95.
pub const DEFAULT_MIN_REQUESTS: u64 = 20;
/// The rendered-page route, served from the render cache (#74).
pub const PAGE_RENDER_ROUTE: &str = "/api/v1/pages/{id}/render";
/// Its p95 budget: the cached-read target, in milliseconds.
pub const PAGE_RENDER_MS: u64 = 50;

/// A budget for one route, or every route under a prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteBudget {
    /// The HTTP method, or every method when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// A route template such as `/v1/pages/{id}`, or a prefix ending in
    /// `*` such as `/v1/export/*`.
    pub route: String,
    /// The p95 budget in milliseconds.
    pub p95_ms: u64,
}

impl RouteBudget {
    fn matches(&self, method: &str, route: &str) -> bool {
        let method_ok = self
            .method
            .as_deref()
            .is_none_or(|m| m.eq_ignore_ascii_case(method));
        let route_ok = self
            .route
            .strip_suffix('*')
            .map_or(self.route == route, |prefix| route.starts_with(prefix));
        method_ok && route_ok
    }

    /// Exact rules beat prefixes, longer prefixes beat shorter ones, and a
    /// rule with a method beats one without.
    fn specificity(&self) -> (bool, usize, bool) {
        (
            !self.route.ends_with('*'),
            self.route.len(),
            self.method.is_some(),
        )
    }
}

/// The budgets for every route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Budgets {
    /// The p95 budget for reads without a rule, in milliseconds.
    pub read_ms: u64,
    /// The p95 budget for writes without a rule, in milliseconds.
    pub write_ms: u64,
    /// Routes with fewer requests in the window are not judged.
    pub min_requests: u64,
    /// Budgets for particular routes.
    pub routes: Vec<RouteBudget>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            read_ms: DEFAULT_READ_MS,
            write_ms: DEFAULT_WRITE_MS,
            min_requests: DEFAULT_MIN_REQUESTS,
            routes: vec![RouteBudget {
                method: Some("GET".to_owned()),
                route: PAGE_RENDER_ROUTE.to_owned(),
                p95_ms: PAGE_RENDER_MS,
            }],
        }
    }
}

/// A route whose p95 is over its budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Breach {
    /// The HTTP method.
    pub method: String,
    /// The route template.
    pub route: String,
    /// The measured p95 in milliseconds.
    pub p95_ms: f64,
    /// The budget in milliseconds.
    pub budget_ms: u64,
    /// Requests in the window.
    pub requests: u64,
}

const fn is_read(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD")
}

impl Budgets {
    /// The p95 budget for a route, in milliseconds.
    #[must_use]
    pub fn budget_for(&self, method: &str, route: &str) -> u64 {
        self.routes
            .iter()
            .filter(|r| r.matches(method, route))
            .max_by_key(|r| r.specificity())
            .map_or_else(
                || {
                    if is_read(method) {
                        self.read_ms
                    } else {
                        self.write_ms
                    }
                },
                |r| r.p95_ms,
            )
    }

    /// Sets each route's budget and over-budget marker.
    pub fn annotate(&self, stats: &mut [RouteStat]) {
        for stat in stats {
            let budget = self.budget_for(&stat.method, &stat.route);
            stat.budget_ms = Some(budget);
            #[expect(clippy::cast_precision_loss, reason = "budgets are small integers")]
            let limit = budget as f64;
            stat.over_budget = stat.counts.requests >= self.min_requests
                && stat.p95_ms.is_some_and(|p95| p95 > limit);
        }
    }

    /// The routes over budget, worst first.
    #[must_use]
    pub fn breaches(&self, stats: &[RouteStat]) -> Vec<Breach> {
        let mut stats = stats.to_vec();
        self.annotate(&mut stats);
        marked_breaches(&stats)
    }

    /// Problems with the budgets, empty when they are valid.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (name, value) in [("read_ms", self.read_ms), ("write_ms", self.write_ms)] {
            if value == 0 {
                problems.push(format!("{name} must be at least 1"));
            }
        }
        for rule in &self.routes {
            if rule.p95_ms == 0 {
                problems.push(format!("the budget for {} must be at least 1", rule.route));
            }
            if !rule.route.starts_with('/') {
                problems.push(format!("the route {} must start with /", rule.route));
            }
        }
        problems
    }
}

/// The routes already marked over budget by [`Budgets::annotate`], worst
/// (highest p95 to budget ratio) first.
#[must_use]
pub fn marked_breaches(stats: &[RouteStat]) -> Vec<Breach> {
    let mut breaches: Vec<Breach> = stats
        .iter()
        .filter(|s| s.over_budget)
        .filter_map(|s| {
            Some(Breach {
                method: s.method.clone(),
                route: s.route.clone(),
                p95_ms: s.p95_ms?,
                budget_ms: s.budget_ms?,
                requests: s.counts.requests,
            })
        })
        .collect();
    breaches.sort_by(|a, b| {
        #[expect(clippy::cast_precision_loss, reason = "budgets are small integers")]
        let ratio = |x: &Breach| x.p95_ms / x.budget_ms.max(1) as f64;
        ratio(b).total_cmp(&ratio(a))
    });
    breaches
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::red::RouteMetrics;

    fn rule(method: Option<&str>, route: &str, p95_ms: u64) -> RouteBudget {
        RouteBudget {
            method: method.map(str::to_owned),
            route: route.to_owned(),
            p95_ms,
        }
    }

    #[test]
    fn rendered_pages_have_the_cached_read_budget_by_default() {
        let budgets = Budgets::default();
        assert_eq!(budgets.budget_for("GET", PAGE_RENDER_ROUTE), 50);
        assert_eq!(
            budgets.budget_for("GET", "/api/v1/pages/{id}"),
            DEFAULT_READ_MS
        );
        assert!(budgets.problems().is_empty());
    }

    #[test]
    fn picks_the_most_specific_rule() {
        let budgets = Budgets {
            routes: vec![
                rule(None, "/v1/export/*", 3_000),
                rule(None, "/v1/export/pdf/*", 5_000),
                rule(Some("post"), "/v1/pages/{id}", 400),
                rule(None, "/v1/pages/{id}", 50),
            ],
            ..Budgets::default()
        };
        assert_eq!(budgets.budget_for("GET", "/v1/other"), DEFAULT_READ_MS);
        assert_eq!(budgets.budget_for("DELETE", "/v1/other"), DEFAULT_WRITE_MS);
        assert_eq!(budgets.budget_for("POST", "/v1/export/docx"), 3_000);
        assert_eq!(budgets.budget_for("POST", "/v1/export/pdf/{id}"), 5_000);
        assert_eq!(budgets.budget_for("GET", "/v1/pages/{id}"), 50);
        assert_eq!(budgets.budget_for("POST", "/v1/pages/{id}"), 400);
        assert!(budgets.problems().is_empty());
    }

    #[test]
    fn marks_slow_routes_with_enough_traffic() {
        let metrics = RouteMetrics::new();
        for _ in 0..30 {
            metrics.record("GET", "/slow", 200, Duration::from_millis(400));
            metrics.record("GET", "/fast", 200, Duration::from_millis(20));
        }
        for _ in 0..5 {
            metrics.record("GET", "/rare", 200, Duration::from_millis(900));
        }
        let budgets = Budgets::default();
        let mut stats = metrics.snapshot(5);
        budgets.annotate(&mut stats);
        let over: Vec<_> = stats
            .iter()
            .filter(|s| s.over_budget)
            .map(|s| s.route.as_str())
            .collect();
        assert_eq!(over, ["/slow"]);
        assert!(stats.iter().all(|s| s.budget_ms == Some(DEFAULT_READ_MS)));
        let breaches = budgets.breaches(&stats);
        let [breach] = breaches.as_slice() else {
            panic!("expected one breach: {breaches:?}");
        };
        assert_eq!(breach.route, "/slow");
        assert_eq!(breach.requests, 30);
        assert!(breach.p95_ms > 300.0);
    }

    #[test]
    fn rejects_invalid_budgets() {
        let budgets = Budgets {
            read_ms: 0,
            routes: vec![rule(None, "pages", 0)],
            ..Budgets::default()
        };
        assert_eq!(budgets.problems().len(), 3);
    }
}
