// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import {
  CLIENT_TELEMETRY_PATH,
  navigationReport,
  navigationSpans,
  resourceReport,
  sendReport,
  serverTraceparent,
  VitalsCollector,
  type ClientReport,
} from "./client-telemetry";

const TP = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

function nav(
  overrides: Partial<Record<string, unknown>> = {},
): PerformanceNavigationTiming {
  return {
    type: "navigate",
    startTime: 0,
    domainLookupStart: 5,
    domainLookupEnd: 15,
    connectStart: 15,
    connectEnd: 40,
    requestStart: 41,
    responseStart: 120,
    responseEnd: 160,
    domContentLoadedEventEnd: 400,
    loadEventEnd: 0,
    serverTiming: [
      { name: "total", description: "" },
      { name: "traceparent", description: TP },
    ],
    ...overrides,
  } as unknown as PerformanceNavigationTiming;
}

function resource(
  duration: number,
  requestStart = 10,
): PerformanceResourceTiming {
  return {
    name: "https://vellum.northstar.dev/api/v1/pages/7?x=1",
    startTime: 5,
    requestStart,
    duration,
    initiatorType: "fetch",
    serverTiming: [{ name: "traceparent", description: TP }],
  } as unknown as PerformanceResourceTiming;
}

describe("client telemetry", () => {
  it("reads the server traceparent only when it is valid", () => {
    expect(serverTraceparent(nav())).toBe(TP);
    expect(
      serverTraceparent(
        nav({ serverTiming: [{ name: "traceparent", description: "x" }] }),
      ),
    ).toBeUndefined();
    expect(serverTraceparent(nav({ serverTiming: undefined }))).toBeUndefined();
  });

  it("turns navigation timing into phases, skipping empty ones", () => {
    const spans = navigationSpans(nav());
    expect(spans.map((s) => s.name)).toEqual([
      "dns",
      "connect",
      "request",
      "response",
      "dom",
    ]);
    expect(spans[2]).toEqual({
      name: "request",
      start_ms: 41,
      duration_ms: 79,
    });
  });

  it("builds a navigation report with TTFB", () => {
    const report = navigationReport(
      nav(),
      { lcp_ms: 900 },
      "/p/1",
      1_700_000_000_000,
    );
    expect(report?.traceparent).toBe(TP);
    expect(report?.request_start_ms).toBe(41);
    expect(report?.vitals).toEqual({ lcp_ms: 900, ttfb_ms: 79 });
    expect(report?.spans[0]?.attributes).toEqual({ type: "navigate" });
    expect(
      navigationReport(nav({ serverTiming: [] }), {}, "/", 0),
    ).toBeUndefined();
    expect(
      navigationReport(nav({ responseStart: 0 }), {}, "/", 0)?.vitals,
    ).toEqual({});
  });

  it("reports only slow, traced API calls", () => {
    const report = resourceReport(resource(1500), "/home", 1);
    expect(report?.spans[0]?.name).toBe("fetch /api/v1/pages/7");
    expect(report?.request_start_ms).toBe(10);
    expect(
      resourceReport(resource(1500, 0), "/home", 1)?.request_start_ms,
    ).toBe(5);
    expect(resourceReport(resource(20), "/home", 1)).toBeUndefined();
  });

  it("collects vitals from observer entries", () => {
    const c = new VitalsCollector();
    const add = (entry: Record<string, unknown>) => {
      c.add(entry as unknown as PerformanceEntry);
    };
    add({ entryType: "largest-contentful-paint", startTime: 700 });
    add({ entryType: "largest-contentful-paint", startTime: 900 });
    add({ entryType: "layout-shift", value: 0.05, hadRecentInput: false });
    add({ entryType: "layout-shift", value: 0.5, hadRecentInput: true });
    add({ entryType: "layout-shift" });
    add({ entryType: "event", duration: 80, interactionId: 3 });
    add({ entryType: "event", duration: 400, interactionId: 0 });
    add({ entryType: "event", duration: 40, interactionId: 4 });
    add({ entryType: "event", duration: 40 });
    add({ entryType: "paint", name: "first-paint", startTime: 100 });
    add({ entryType: "paint", name: "first-contentful-paint", startTime: 150 });
    add({ entryType: "mark", startTime: 1 });
    expect(c.snapshot()).toEqual({
      lcp_ms: 900,
      cls: 0.05,
      inp_ms: 80,
      fcp_ms: 150,
    });
  });

  it("observes every supported entry type and disconnects", () => {
    const observed: string[] = [];
    const disconnect = vi.fn();
    let callback: PerformanceObserverCallback | undefined;
    class FakeObserver {
      constructor(observerCallback: PerformanceObserverCallback) {
        callback = observerCallback;
      }
      observe(options: PerformanceObserverInit) {
        if (options.type === "event") {
          throw new Error("unsupported");
        }
        observed.push(options.type ?? "");
      }
      disconnect = disconnect;
    }
    const c = new VitalsCollector().observe(FakeObserver);
    expect(observed).toEqual([
      "largest-contentful-paint",
      "layout-shift",
      "paint",
    ]);
    callback?.(
      {
        getEntries: () => [
          { entryType: "paint", name: "first-contentful-paint", startTime: 9 },
        ],
      } as unknown as PerformanceObserverEntryList,
      {} as PerformanceObserver,
    );
    expect(c.snapshot().fcp_ms).toBe(9);
    c.disconnect();
    expect(disconnect).toHaveBeenCalledTimes(3);
    expect(new VitalsCollector().observe().snapshot()).toEqual({});
  });

  it("posts reports and never throws", async () => {
    const report: ClientReport = {
      traceparent: TP,
      page: "/",
      request_start_ms: 1,
      time_origin_ms: 1,
      spans: [],
      vitals: {},
    };
    const ok = vi.fn(() =>
      Promise.resolve(new Response(undefined, { status: 202 })),
    );
    expect(await sendReport(report, ok, "https://x")).toBe(true);
    expect(ok).toHaveBeenCalledWith(
      `https://x${CLIENT_TELEMETRY_PATH}`,
      expect.objectContaining({ keepalive: true }),
    );
    const failing = vi.fn(() => Promise.reject(new Error("offline")));
    expect(await sendReport(report, failing)).toBe(false);
  });
});
