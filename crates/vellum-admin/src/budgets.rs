// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum budgets`: fail when a route's p95 is over its budget.
//!
//! Run it after a load test, against the server under test: it reads the
//! per-route figures (`GET /api/v1/admin/routes`, admin scope) and exits
//! non-zero when any route with enough traffic is over budget. The budgets
//! are the server's own settings unless a budgets file is given. It can
//! also check a routes file saved earlier, so CI can keep the evidence.

use std::path::PathBuf;

use vellum_kernel::{Error, ErrorKind, Result};
use vellum_observe::budget::marked_breaches;
use vellum_observe::{Breach, Budgets};

use crate::service::RoutesView;

/// The routes endpoint, relative to the server URL.
pub const ROUTES_PATH: &str = "/api/v1/admin/routes";

/// Arguments of `vellum budgets`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct BudgetsArgs {
    /// The server, such as `https://vellum.northstar.dev`.
    #[arg(long, env = "VELLUM_URL", default_value = "http://127.0.0.1:8080")]
    pub server: String,
    /// An API token with the `admin` scope.
    #[arg(long, env = "VELLUM_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    /// The window to judge, in minutes (1 to 60).
    #[arg(long, default_value_t = 15)]
    pub minutes: u64,
    /// Check a routes file (the endpoint's JSON) instead of asking a server.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// Budgets to check against (JSON) instead of the server's settings.
    #[arg(long)]
    pub budgets: Option<PathBuf>,
}

/// What the check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// Routes judged (enough requests in the window).
    pub judged: usize,
    /// Routes over budget, worst first.
    pub breaches: Vec<Breach>,
}

impl Verdict {
    /// Whether every judged route is within budget.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.breaches.is_empty()
    }
}

fn read_json<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
    what: &'static str,
) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, what).with_source(e))?;
    serde_json::from_str(&text)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, what).with_source(e))
}

/// Runs the check.
///
/// # Errors
///
/// Returns an error when a file cannot be read, the budgets are invalid,
/// or the server cannot be reached or refuses.
pub fn run(args: &BudgetsArgs) -> Result<Verdict> {
    let view: RoutesView = if let Some(path) = &args.input {
        read_json(path, "cannot read the routes file")?
    } else {
        let minutes = args.minutes.to_string();
        crate::http::get_json(
            &args.server,
            args.token.as_deref(),
            ROUTES_PATH,
            &[("minutes", minutes.as_str())],
        )?
    };
    let mut routes = view.routes;
    // Without a budgets file, trust the marks the server set from its own
    // settings; with one, judge again.
    let min_requests = if let Some(path) = &args.budgets {
        let budgets: Budgets = read_json(path, "cannot read the budgets file")?;
        let problems = budgets.problems();
        if !problems.is_empty() {
            return Err(Error::new(ErrorKind::InvalidInput, problems.join("; ")));
        }
        budgets.annotate(&mut routes);
        budgets.min_requests
    } else {
        vellum_observe::budget::DEFAULT_MIN_REQUESTS
    };
    Ok(Verdict {
        judged: routes
            .iter()
            .filter(|r| r.counts.requests >= min_requests)
            .count(),
        breaches: marked_breaches(&routes),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use vellum_observe::{RouteMetrics, TraceStore};

    use super::*;

    fn routes_file(dir: &std::path::Path, budgets: &Budgets) -> PathBuf {
        let metrics = RouteMetrics::new();
        for _ in 0..25 {
            metrics.record("GET", "/v1/slow", 200, Duration::from_millis(400));
            metrics.record("POST", "/v1/ok", 200, Duration::from_millis(100));
        }
        let mut routes = metrics.snapshot(5);
        budgets.annotate(&mut routes);
        let view = RoutesView {
            minutes: 5,
            routes,
            in_flight: 0,
            process: vellum_observe::process::snapshot(),
            buffer: TraceStore::new(10, 1 << 20).stats(),
        };
        let path = dir.join("routes.json");
        std::fs::write(&path, serde_json::to_vec(&view).unwrap()).unwrap();
        path
    }

    fn args(input: PathBuf) -> BudgetsArgs {
        BudgetsArgs {
            server: "http://127.0.0.1:9".into(),
            token: None,
            minutes: 15,
            input: Some(input),
            budgets: None,
        }
    }

    #[test]
    fn fails_on_routes_the_server_marked() {
        let dir = tempfile::tempdir().unwrap();
        let verdict = run(&args(routes_file(dir.path(), &Budgets::default()))).unwrap();
        assert_eq!(verdict.judged, 2);
        assert!(!verdict.passed());
        assert_eq!(verdict.breaches.first().unwrap().route, "/v1/slow");
    }

    #[test]
    fn judges_again_with_a_budgets_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = args(routes_file(dir.path(), &Budgets::default()));
        let file = dir.path().join("budgets.json");
        std::fs::write(&file, r#"{"routes":[{"route":"/v1/slow","p95_ms":1000}]}"#).unwrap();
        a.budgets = Some(file.clone());
        assert!(run(&a).unwrap().passed());
        std::fs::write(&file, r#"{"read_ms":0}"#).unwrap();
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::InvalidInput);
        a.input = Some(dir.path().join("missing.json"));
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn reports_an_unreachable_server() {
        let mut a = args(PathBuf::new());
        a.input = None;
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::Unavailable);
        a.server = "not a url".into();
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::InvalidInput);
    }
}
