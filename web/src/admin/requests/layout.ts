// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Field, SpanEvent, Trace } from "./api";

/** One row of the waterfall. */
export interface WaterfallRow {
  readonly key: string;
  readonly name: string;
  readonly lane: "server" | "browser";
  readonly depth: number;
  /** Offset from the trace start, in microseconds (negative for browser work before the request). */
  readonly startUs: number;
  readonly durationUs: number;
  /** Bar position and width as percentages of the whole timeline. */
  readonly leftPct: number;
  readonly widthPct: number;
  readonly error: boolean;
  readonly fields: readonly Field[];
  readonly events: readonly SpanEvent[];
}

/** Lays a trace out as rows: browser spans first, then server spans depth-first in start order. */
export function waterfallRows(trace: Trace): WaterfallRow[] {
  const ROOT = -1;
  const children = new Map<number, Trace["spans"][number][]>();
  for (const span of trace.spans) {
    const parent = span.parent ?? ROOT;
    const siblings = children.get(parent) ?? [];
    siblings.push(span);
    children.set(parent, siblings);
  }
  for (const siblings of children.values()) {
    siblings.sort((a, b) => a.start_us - b.start_us || a.index - b.index);
  }
  const starts = [0, ...trace.client.map((c) => c.start_us)];
  const ends = [
    trace.duration_us,
    ...trace.spans.map((s) => s.start_us + s.duration_us),
    ...trace.client.map((c) => c.start_us + c.duration_us),
  ];
  const origin = Math.min(...starts);
  const total = Math.max(1, Math.max(...ends) - origin);
  const place = (start: number, duration: number) => ({
    leftPct: ((start - origin) / total) * 100,
    widthPct: Math.max(0.2, (duration / total) * 100),
  });

  const rows: WaterfallRow[] = trace.client.map((c, index) => ({
    key: `c${String(index)}`,
    name: c.name,
    lane: "browser",
    depth: 0,
    startUs: c.start_us,
    durationUs: c.duration_us,
    ...place(c.start_us, c.duration_us),
    error: false,
    fields: c.attributes,
    events: [],
  }));
  const known = new Set(trace.spans.map((s) => s.index));
  const visit = (parent: number, depth: number) => {
    for (const span of children.get(parent) ?? []) {
      rows.push({
        key: `s${String(span.index)}`,
        name: span.name,
        lane: "server",
        depth,
        startUs: span.start_us,
        durationUs: span.duration_us,
        ...place(span.start_us, span.duration_us),
        error: span.error,
        fields: span.fields,
        events: span.events,
      });
      visit(span.index, depth + 1);
    }
  };
  visit(ROOT, 0);
  // Spans whose parent was dropped by the per-trace cap still show, at the top level.
  for (const [parent] of children) {
    if (parent !== ROOT && !known.has(parent)) {
      visit(parent, 0);
    }
  }
  return rows;
}

/** A duration for people: microseconds, milliseconds or seconds. */
export function formatDuration(us: number): string {
  if (us < 1000) return `${String(Math.round(us))} µs`;
  if (us < 1_000_000) return `${(us / 1000).toFixed(1)} ms`;
  return `${(us / 1_000_000).toFixed(2)} s`;
}
