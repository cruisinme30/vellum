// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { TRACE } from "./fixtures";
import { formatDuration, waterfallRows } from "./layout";

describe("waterfall layout", () => {
  it(
    "orders browser spans, then the server tree depth-first, then " + "orphans",
    () => {
      const rows = waterfallRows(TRACE);
      expect(rows.map((r) => `${r.lane}:${r.name}:${String(r.depth)}`)).toEqual(
        [
          "browser:navigation:0",
          "server:http.request:0",
          "server:authz.check:1",
          "server:db.query:2",
          "server:orphan:0",
        ],
      );
    },
  );

  it(
    "scales bars to the whole timeline, including browser time before the " +
      "request",
    () => {
      const rows = waterfallRows(TRACE);
      const nav = rows[0];
      const root = rows[1];
      expect(nav?.leftPct).toBe(0);
      expect(nav?.widthPct).toBe(100);
      expect(root?.leftPct).toBeCloseTo(20);
      expect(root?.widthPct).toBeCloseTo(60);
      const tiny = waterfallRows({
        ...TRACE,
        client: [],
        spans: [{ ...TRACE.spans[0], duration_us: 0 }] as typeof TRACE.spans,
      });
      expect(tiny[0]?.widthPct).toBeCloseTo(0.2);
    },
  );

  it("formats durations", () => {
    expect(formatDuration(850)).toBe("850 µs");
    expect(formatDuration(12_340)).toBe("12.3 ms");
    expect(formatDuration(1_240_000)).toBe("1.24 s");
  });
});
