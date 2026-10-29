// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum profile` and `vellum budgets`: admin commands that ask a running
//! server (ADR 0021).

use std::process::ExitCode;

use vellum_admin::budgets::{self, BudgetsArgs};
use vellum_admin::profile::{self, ProfileArgs};

/// Writes a flame graph of a running server.
pub fn profile(args: &ProfileArgs) -> ExitCode {
    match profile::run(args) {
        Ok(path) => {
            tracing::info!(path = %path.display(), "profile written");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "profile failed");
            ExitCode::FAILURE
        }
    }
}

/// Fails when any route is over its p95 budget.
pub fn budgets(args: &BudgetsArgs) -> ExitCode {
    match budgets::run(args) {
        Ok(verdict) if verdict.passed() => {
            tracing::info!(judged = verdict.judged, "every route is within budget");
            ExitCode::SUCCESS
        }
        Ok(verdict) => {
            for b in &verdict.breaches {
                tracing::error!(
                    method = %b.method,
                    route = %b.route,
                    p95_ms = b.p95_ms,
                    budget_ms = b.budget_ms,
                    requests = b.requests,
                    "route over budget"
                );
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            tracing::error!(%error, "budget check failed");
            ExitCode::FAILURE
        }
    }
}
