// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Trace, TraceSummary } from "./api";

/** A trace with a browser span, a nested server tree and an orphan. */
export const TRACE: Trace = {
  trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
  started_at_ms: 1_767_225_600_000,
  duration_us: 12_000,
  spans: [
    {
      index: 0,
      name: "http.request",
      target: "vellum_api",
      start_us: 0,
      duration_us: 12_000,
      fields: [
        ["http.route", "/api/v1/pages/{page_id}"],
        ["http.response.status_code", "200"],
      ],
      events: [],
      error: false,
    },
    {
      index: 2,
      parent: 1,
      name: "db.query",
      target: "vellum_db",
      start_us: 3000,
      duration_us: 4000,
      fields: [["db.statement", "SELECT 1"]],
      events: [{ offset_us: 3500, level: "WARN", message: "slow plan" }],
      error: true,
    },
    {
      index: 1,
      parent: 0,
      name: "authz.check",
      target: "vellum_authz",
      start_us: 1000,
      duration_us: 8000,
      fields: [],
      events: [],
      error: false,
    },
    {
      index: 9,
      parent: 7,
      name: "orphan",
      target: "vellum_x",
      start_us: 100,
      duration_us: 10,
      fields: [],
      events: [],
      error: false,
    },
  ],
  client: [
    { name: "navigation", start_us: -4000, duration_us: 20_000, attributes: [["page", "/p/{id}"]] },
  ],
  dropped_spans: 2,
};

/** One summary row for {@link TRACE}. */
export const SUMMARY: TraceSummary = {
  seq: 41,
  trace_id: TRACE.trace_id,
  started_at_ms: TRACE.started_at_ms,
  duration_us: 12_000,
  name: "http.request",
  route: "/api/v1/pages/{page_id}",
  method: "GET",
  status: 200,
  request_id: "req-1",
  error: false,
  span_count: 4,
  has_client: true,
};
