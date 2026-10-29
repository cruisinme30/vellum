// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Metrics for My Work, through the global OpenTelemetry meter that
//! `vellum_kernel::telemetry::init` installs. Spans are emitted with
//! `tracing` where the work happens.

use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};

/// Tasks projected from checklist files.
pub const PROJECTED_TOTAL: &str = "vellum.tasks.projected";
/// Quick-add parses, by whether a date was found.
pub const QUICK_ADD_TOTAL: &str = "vellum.tasks.quick_add";
/// Calendar documents rendered (the ICS feed and CalDAV), by kind.
pub const CALENDAR_RENDERS_TOTAL: &str = "vellum.tasks.calendar.renders";
/// Size of rendered calendar documents in bytes.
pub const CALENDAR_BYTES: &str = "vellum.tasks.calendar.bytes";
/// Items imported, by source and outcome.
pub const IMPORTED_TOTAL: &str = "vellum.tasks.imported";
/// Captures from e-mail and Slack, by channel and outcome.
pub const CAPTURES_TOTAL: &str = "vellum.tasks.captures";
/// Reminders found due.
pub const REMINDERS_TOTAL: &str = "vellum.tasks.reminders";

struct Instruments {
    projected: Counter<u64>,
    quick_add: Counter<u64>,
    calendar_renders: Counter<u64>,
    calendar_bytes: Histogram<u64>,
    imported: Counter<u64>,
    captures: Counter<u64>,
    reminders: Counter<u64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter = opentelemetry::global::meter("vellum");
        Instruments {
            projected: meter
                .u64_counter(PROJECTED_TOTAL)
                .with_description("Tasks projected from checklist files")
                .build(),
            quick_add: meter
                .u64_counter(QUICK_ADD_TOTAL)
                .with_description("Quick-add parses")
                .build(),
            calendar_renders: meter
                .u64_counter(CALENDAR_RENDERS_TOTAL)
                .with_description("Calendar documents rendered")
                .build(),
            calendar_bytes: meter
                .u64_histogram(CALENDAR_BYTES)
                .with_description("Size of rendered calendar documents")
                .with_unit("By")
                .build(),
            imported: meter
                .u64_counter(IMPORTED_TOTAL)
                .with_description("Tasks imported from other apps")
                .build(),
            captures: meter
                .u64_counter(CAPTURES_TOTAL)
                .with_description("Tasks captured from e-mail and Slack")
                .build(),
            reminders: meter
                .u64_counter(REMINDERS_TOTAL)
                .with_description("Reminders found due")
                .build(),
        }
    })
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

pub(crate) fn record_projected(n: usize) {
    instruments().projected.add(count(n), &[]);
}

pub(crate) fn record_quick_add(dated: bool) {
    instruments()
        .quick_add
        .add(1, &[KeyValue::new("dated", dated)]);
}

pub(crate) fn record_calendar(kind: &'static str, bytes: usize) {
    let i = instruments();
    i.calendar_renders.add(1, &[KeyValue::new("kind", kind)]);
    i.calendar_bytes
        .record(count(bytes), &[KeyValue::new("kind", kind)]);
}

pub(crate) fn record_import(source: &'static str, outcome: &'static str, n: usize) {
    instruments().imported.add(
        count(n),
        &[
            KeyValue::new("source", source),
            KeyValue::new("outcome", outcome),
        ],
    );
}

pub(crate) fn record_capture(channel: &'static str, outcome: &'static str) {
    instruments().captures.add(
        1,
        &[
            KeyValue::new("channel", channel),
            KeyValue::new("outcome", outcome),
        ],
    );
}

pub(crate) fn record_reminders(n: usize) {
    instruments().reminders.add(count(n), &[]);
}
