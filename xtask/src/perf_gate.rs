// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The nightly performance gate: a p95 that grows by 10 % or more fails.
//!
//! Both files share one shape, written by the k6 suite's `handleSummary`
//! (`deploy/perf/k6/summary.js`):
//!
//! ```json
//! { "scenarios": { "read_cached": { "p95_ms": 41.2 }, "write": { "p95_ms": 88.0 } } }
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

/// The p95 growth over the baseline at which the gate fails.
pub const DEFAULT_TOLERANCE: f64 = 0.10;

/// The p95 latencies of one k6 run, by scenario.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Summary {
    /// Scenario name to its measurements.
    pub scenarios: BTreeMap<String, Scenario>,
}

/// One scenario's measurement.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Scenario {
    /// The 95th percentile request duration, in milliseconds.
    pub p95_ms: f64,
}

/// Parses `--baseline <file> --current <file> [--tolerance <ratio>]`, reads
/// both files and runs the gate.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut baseline = None;
    let mut current = None;
    let mut tolerance = DEFAULT_TOLERANCE;
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let value = iter.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--baseline" => baseline = Some(value.clone()),
            "--current" => current = Some(value.clone()),
            "--tolerance" => {
                tolerance = value
                    .parse()
                    .map_err(|e| format!("--tolerance {value}: {e}"))?;
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let baseline = load(Path::new(&baseline.ok_or("--baseline is required")?))?;
    let current = load(Path::new(&current.ok_or("--current is required")?))?;
    let report = compare(&baseline, &current, tolerance)?;
    #[expect(clippy::print_stdout, reason = "xtask reports to the terminal")]
    {
        print!("{report}");
    }
    Ok(())
}

fn load(path: &Path) -> Result<Summary, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not a k6 summary: {e}", path.display()))
}

/// Compares every baseline scenario with the current run.
///
/// Returns a table for the log, or an error listing each scenario that
/// regressed by `tolerance` or more, or is missing from the current run.
/// Scenarios that are new in the current run pass; they join the baseline
/// when it is next updated.
pub fn compare(baseline: &Summary, current: &Summary, tolerance: f64) -> Result<String, String> {
    let mut report = String::new();
    let mut failures = Vec::new();
    for (name, base) in &baseline.scenarios {
        let Some(now) = current.scenarios.get(name) else {
            failures.push(format!("{name}: missing from the current run"));
            continue;
        };
        let change = if base.p95_ms > 0.0 {
            now.p95_ms / base.p95_ms - 1.0
        } else {
            0.0
        };
        // The epsilon keeps an exact 10 % from passing on rounding.
        let regressed = change >= tolerance - 1e-9;
        let verdict = if regressed { "REGRESSED" } else { "ok" };
        let _ = writeln!(
            report,
            "{name:<24} baseline {:>9.1} ms  now {:>9.1} ms  {:>+7.1} %  {verdict}",
            base.p95_ms,
            now.p95_ms,
            change * 100.0
        );
        if regressed {
            failures.push(format!(
                "{name}: p95 {:.1} ms is {:.1} % over the baseline {:.1} ms (limit {:.0} %)",
                now.p95_ms,
                change * 100.0,
                base.p95_ms,
                tolerance * 100.0
            ));
        }
    }
    if failures.is_empty() {
        Ok(report)
    } else {
        Err(format!(
            "{report}\nPerformance gate failed:\n  {}",
            failures.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(entries: &[(&str, f64)]) -> Summary {
        Summary {
            scenarios: entries
                .iter()
                .map(|(name, p95_ms)| ((*name).to_owned(), Scenario { p95_ms: *p95_ms }))
                .collect(),
        }
    }

    #[test]
    fn within_tolerance_passes() {
        let base = summary(&[("read", 100.0), ("write", 200.0)]);
        let now = summary(&[("read", 109.9), ("write", 150.0)]);
        assert!(compare(&base, &now, DEFAULT_TOLERANCE).is_ok());
    }

    #[test]
    fn ten_percent_or_more_fails() {
        let base = summary(&[("read", 100.0), ("write", 30.0)]);
        let now = summary(&[("read", 110.5), ("write", 33.0)]);
        let error = compare(&base, &now, DEFAULT_TOLERANCE).unwrap_err();
        assert!(error.contains("read: p95 110.5 ms is 10.5 %"), "{error}");
        assert!(error.contains("write: p95 33.0 ms is 10.0 %"), "{error}");
    }

    #[test]
    fn a_missing_scenario_fails_and_a_new_one_passes() {
        let base = summary(&[("read", 100.0)]);
        let now = summary(&[("collab", 10.0)]);
        let error = compare(&base, &now, DEFAULT_TOLERANCE).unwrap_err();
        assert!(error.contains("read: missing"), "{error}");
        assert!(compare(&summary(&[]), &now, DEFAULT_TOLERANCE).is_ok());
    }

    #[test]
    fn parses_the_k6_summary_shape() {
        let parsed: Summary =
            serde_json::from_str(r#"{"scenarios":{"read_cached":{"p95_ms":41.2}}}"#).unwrap();
        assert_eq!(parsed, summary(&[("read_cached", 41.2)]));
    }

    #[test]
    fn the_committed_baseline_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deploy/perf/baseline.json");
        let baseline = load(&path).unwrap();
        assert!(!baseline.scenarios.is_empty());
    }

    #[test]
    fn flags_are_checked() {
        assert!(run(&["--baseline".to_owned()]).is_err());
        assert!(run(&["--nope".to_owned(), "x".to_owned()]).is_err());
        assert!(run(&[]).is_err());
    }
}
