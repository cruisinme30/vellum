// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for trace context parsing, latency windows and folding.

use std::time::Duration;

use proptest::prelude::*;
use vellum_observe::flame;
use vellum_observe::{RouteMetrics, TraceParent};

proptest! {
    #[test]
    fn traceparent_parsing_never_panics(s in "\\PC{0,80}") {
        let _ = s.parse::<TraceParent>();
    }

    #[test]
    fn valid_traceparents_round_trip(trace in "[0-9a-f]{32}", span in "[0-9a-f]\
        {16}", sampled: bool) {
        prop_assume!(
            trace.chars().any(|c| c != '0') && span.chars().any(|c| c != '0')
        );
        let header = format!(
            "00-{trace}-{span}-{}",
            if sampled { "01" } else { "00" }
        );
        let parsed: TraceParent = header.parse().unwrap();
        prop_assert_eq!(parsed.to_string(), header);
    }

    #[test]
    fn quantiles_are_monotonic(
        latencies in proptest::collection::vec(0_u64..40_000, 1..200)
    ) {
        // A fixed minute: with the wall clock, recording could cross into
        // the next minute and a one-minute snapshot would miss requests.
        const MINUTE: u64 = 29_000_000;
        let m = RouteMetrics::new();
        for ms in &latencies {
            m.record_at(MINUTE, "GET", "/p", 200, Duration::from_millis(*ms));
        }
        let stat = m.snapshot_at(MINUTE, 1).pop().unwrap();
        let p50 = stat.p50_ms.unwrap();
        let p95 = stat.p95_ms.unwrap();
        let p99 = stat.p99_ms.unwrap();
        prop_assert!(p50 <= p95 && p95 <= p99);
        prop_assert_eq!(stat.counts.requests, latencies.len() as u64);
    }

    #[test]
    fn folded_text_round_trips(
        stacks in proptest::collection::btree_map(
            "[a-z]{1,8}(;[a-z]{1,8}){0,4}",
            1_u64..1_000_000,
            0..20
        )
    ) {
        let folded: flame::Folded = stacks;
        prop_assert_eq!(flame::from_text(&flame::to_text(&folded)), folded);
    }
}
