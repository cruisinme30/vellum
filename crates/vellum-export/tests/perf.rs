// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The print PDF budget (NFR): a 30-page document renders in under 3 s at
//! p95. The budget holds for optimised builds
//! (`cargo test --release -p vellum-export --test perf`); unoptimised
//! test builds only guard against gross regressions.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use vellum_export::pdf::{PdfEngine, PrintTheme};
use vellum_export::{LowerOptions, Ports, Target, lower};
use vellum_jobs::export::CollectionsEngine;

const RUNS: usize = 7;

fn thirty_pages() -> String {
    let guide = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/guide/page");
    let mut out =
        String::from("---\ntitle: Northstar operations handbook\n---\n\n::: toc\n:::\n\n");
    let mut names: Vec<_> = std::fs::read_dir(&guide)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "md")
                && !p.to_string_lossy().ends_with(".canonical.md")
        })
        .collect();
    names.sort();
    for round in 1..=6 {
        for path in &names {
            let text = std::fs::read_to_string(path).unwrap();
            let body = text
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n"))
                .map_or(text.as_str(), |(_, body)| body);
            let _ = write!(out, "\n# Part {round}\n\n");
            out.push_str(&body.replace("\n# ", "\n## "));
            out.push('\n');
        }
    }
    out
}

/// Ports with the collections engine, so calculators show their values.
fn ports() -> Ports {
    Ports {
        collections: std::sync::Arc::new(CollectionsEngine),
        ..Ports::default()
    }
}

#[tokio::test]
async fn thirty_pages_render_within_budget() {
    let doc = vellum_format::parse(&thirty_pages()).document;
    let lowered = lower(
        &doc,
        &ports(),
        &LowerOptions::new(Target::Print, "2026-10-29"),
    )
    .await;
    let engine = PdfEngine::new();
    let theme = PrintTheme::default();
    let mut times = Vec::with_capacity(RUNS);
    let mut pages = 0;
    for _ in 0..RUNS {
        let started = Instant::now();
        let pdf = engine.render(&lowered.flow, &theme).unwrap();
        times.push(started.elapsed());
        pages = pdf.pages.len();
    }
    times.sort();
    // With 7 samples the 95th percentile is the slowest run.
    let p95 = *times.last().unwrap();
    assert!(pages >= 30, "the fixture is only {pages} pages");
    eprintln!("{pages} pages, p95 {p95:?}, runs {times:?}");
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(3)
    };
    assert!(
        p95 < budget,
        "{pages} pages: p95 {p95:?} over the {budget:?} budget (runs {times:?})"
    );
}
