// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Process-level saturation: resident memory and CPU time.
//!
//! The single binary's targets (brief §12: under 80 MB idle, under 1 % CPU
//! idle) are checked against these figures. They are read from `/proc` on
//! Linux; on other platforms they are `None`, and the OpenTelemetry
//! collector's host metrics fill the gap in deployments that run one.

use std::sync::OnceLock;

use opentelemetry::metrics::ObservableGauge;
use serde::{Deserialize, Serialize};

/// A snapshot of the process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessSnapshot {
    /// Resident set size in bytes.
    pub rss_bytes: Option<u64>,
    /// User plus system CPU time in milliseconds since start.
    pub cpu_ms: Option<u64>,
    /// Number of threads.
    pub threads: Option<u64>,
}

/// Reads the current snapshot.
#[must_use]
pub fn snapshot() -> ProcessSnapshot {
    std::fs::read_to_string("/proc/self/stat")
        .map(|stat| parse_stat(&stat))
        .unwrap_or_default()
}

/// Parses `/proc/self/stat`. Fields after the parenthesised command name
/// are space-separated; utime and stime are fields 14 and 15, threads 20
/// and rss (in pages) 24, counting from 1.
fn parse_stat(stat: &str) -> ProcessSnapshot {
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return ProcessSnapshot::default();
    };
    // `rest` starts at field 3 (state).
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |n: usize| fields.get(n - 3).and_then(|v| v.parse::<u64>().ok());
    // Linux reports clock ticks at 100 Hz and 4 KiB pages on every
    // architecture Vellum ships for; reading the exact values needs libc.
    let cpu_ms = field(14).zip(field(15)).map(|(u, s)| (u + s) * 10);
    ProcessSnapshot {
        rss_bytes: field(24).map(|pages| pages * 4096),
        cpu_ms,
        threads: field(20),
    }
}

static GAUGES: OnceLock<(ObservableGauge<u64>, ObservableGauge<u64>)> = OnceLock::new();

/// Registers `process.memory.usage` and `process.thread.count` gauges on
/// the global meter provider. Safe to call more than once.
pub fn register_metrics() {
    let _ = GAUGES.get_or_init(|| {
        let meter = opentelemetry::global::meter("vellum-observe");
        let memory = meter
            .u64_observable_gauge("process.memory.usage")
            .with_unit("By")
            .with_description("Resident memory of the Vellum process")
            .with_callback(|observer| {
                if let Some(rss) = snapshot().rss_bytes {
                    observer.observe(rss, &[]);
                }
            })
            .build();
        let threads = meter
            .u64_observable_gauge("process.thread.count")
            .with_unit("{thread}")
            .with_description("Threads in the Vellum process")
            .with_callback(|observer| {
                if let Some(n) = snapshot().threads {
                    observer.observe(n, &[]);
                }
            })
            .build();
        (memory, threads)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat() {
        let stat = "4242 (vellum (srv)) S 1 4242 4242 0 -1 4194560 1000 0 0 0 150 50 0 0 20 0 7 0 100 81920000 2048 18446744073709551615";
        let s = parse_stat(stat);
        assert_eq!(s.cpu_ms, Some(2000));
        assert_eq!(s.threads, Some(7));
        assert_eq!(s.rss_bytes, Some(2048 * 4096));
        assert_eq!(parse_stat("garbage"), ProcessSnapshot::default());
    }

    #[test]
    fn reads_this_process() {
        let s = snapshot();
        if cfg!(target_os = "linux") {
            assert!(s.rss_bytes.unwrap_or(0) > 0);
        }
        register_metrics();
        register_metrics();
    }
}
