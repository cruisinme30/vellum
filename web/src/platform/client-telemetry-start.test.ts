// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { ClientReport } from "./client-telemetry";
import { startClientTelemetry, type TelemetryEnvironment } from "./client-telemetry-start";

const TP = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

function environment(withObserver: boolean) {
  const sent: ClientReport[] = [];
  const listeners = new Map<string, () => void>();
  const callbacks: PerformanceObserverCallback[] = [];
  class FakeObserver {
    constructor(observerCallback: PerformanceObserverCallback) {
      callbacks.push(observerCallback);
    }
    observe() {
      // Accepts every type.
    }
    disconnect() {
      // Nothing to release.
    }
  }
  const document = {
    visibilityState: "visible" as DocumentVisibilityState,
    addEventListener: (name: string, listener: () => void) => listeners.set(name, listener),
    removeEventListener: (name: string) => listeners.delete(name),
  };
  const telemetryEnvironment = {
    performance: {
      timeOrigin: 1000,
      getEntriesByType: () => [
        {
          type: "navigate",
          requestStart: 10,
          responseStart: 30,
          responseEnd: 40,
          domContentLoadedEventEnd: 90,
          serverTiming: [{ name: "traceparent", description: TP }],
        },
      ],
    },
    location: { pathname: "/home" },
    document,
    Observer: withObserver ? (FakeObserver as unknown as typeof PerformanceObserver) : undefined,
    send: (r: ClientReport) => {
      sent.push(r);
      return Promise.resolve(true);
    },
  } as unknown as TelemetryEnvironment;
  return { env: telemetryEnvironment, sent, listeners, callbacks, document };
}

describe("startClientTelemetry", () => {
  it("sends the page view once when the page is hidden", () => {
    const { env, sent, listeners, document } = environment(false);
    const stop = startClientTelemetry(env);
    listeners.get("visibilitychange")?.();
    expect(sent).toHaveLength(0);
    document.visibilityState = "hidden";
    listeners.get("visibilitychange")?.();
    listeners.get("visibilitychange")?.();
    expect(sent).toHaveLength(1);
    expect(sent[0]?.page).toBe("/home");
    expect(sent[0]?.time_origin_ms).toBe(1000);
    stop();
    expect(listeners.size).toBe(0);
  });

  it("reports slow resources as they finish", () => {
    const { env, sent, callbacks } = environment(true);
    const stop = startClientTelemetry(env);
    const resourceCallback = callbacks.at(-1);
    resourceCallback?.(
      {
        getEntries: () => [
          {
            name: "/api/v1/x",
            startTime: 1,
            requestStart: 2,
            duration: 2000,
            initiatorType: "fetch",
            serverTiming: [{ name: "traceparent", description: TP }],
          },
          { name: "/api/v1/y", startTime: 1, requestStart: 2, duration: 5, initiatorType: "fetch" },
        ],
      } as unknown as PerformanceObserverEntryList,
      {} as PerformanceObserver,
    );
    expect(sent.map((r) => r.spans[0]?.name)).toEqual(["fetch /api/v1/x"]);
    stop();
  });

  it("skips pages without navigation timing", () => {
    const { env, sent, listeners, document } = environment(false);
    startClientTelemetry({
      ...env,
      performance: { timeOrigin: 0, getEntriesByType: () => [] },
    });
    document.visibilityState = "hidden";
    listeners.get("visibilitychange")?.();
    expect(sent).toHaveLength(0);
  });

  it("uses the real sender by default", () => {
    const fetchSpy = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response(undefined, { status: 202 }));
    const { env, listeners, document } = environment(false);
    const withoutSend = { ...env, send: undefined } as unknown as TelemetryEnvironment;
    startClientTelemetry(withoutSend);
    document.visibilityState = "hidden";
    listeners.get("visibilitychange")?.();
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    fetchSpy.mockRestore();
  });
});
