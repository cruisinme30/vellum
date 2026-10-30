// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Client telemetry joined to server traces (brief §12).
 *
 * The server puts its W3C `traceparent` in the `Server-Timing` header of
 * every response, and the browser exposes it on the navigation and resource
 * timing entries. This module reads it, measures what only the browser can
 * see (navigation phases, Core Web Vitals, slow API calls) and posts the
 * measurements back with that trace id, so Admin → Requests shows the
 * browser's part of a request next to the server's spans.
 */

/** Where reports go, relative to the server origin. */
export const CLIENT_TELEMETRY_PATH = "/api/v1/telemetry/client";

/** API calls slower than this are reported, in milliseconds. */
const SLOW_FETCH_MS = 1000;

/** One measured span, on the page's performance time origin. */
export interface ClientSpan {
  readonly name: string;
  readonly start_ms: number;
  readonly duration_ms: number;
  readonly attributes?: Readonly<Record<string, string>>;
}

/** Core Web Vitals and page metrics, in milliseconds except CLS. */
export interface Vitals {
  lcp_ms?: number;
  inp_ms?: number;
  cls?: number;
  ttfb_ms?: number;
  fcp_ms?: number;
}

/** The body posted to {@link CLIENT_TELEMETRY_PATH}. */
export interface ClientReport {
  readonly traceparent: string;
  readonly page: string;
  readonly request_start_ms: number;
  readonly time_origin_ms: number;
  readonly spans: readonly ClientSpan[];
  readonly vitals: Vitals;
}

/** The parts of a timing entry this module reads. */
interface TimedEntry {
  readonly startTime: number;
  readonly requestStart: number;
  readonly serverTiming?: readonly {
    readonly name: string;
    readonly description: string;
  }[];
}

const TRACEPARENT = /^00-[\da-f]{32}-[\da-f]{16}-[\da-f]{2}$/;

/** The server trace of a navigation or fetch, from its `Server-Timing`. */
export function serverTraceparent(entry: TimedEntry): string | undefined {
  const value = entry.serverTiming?.find(
    (metric) => metric.name === "traceparent",
  )?.description;
  return value !== undefined && TRACEPARENT.test(value) ? value : undefined;
}

function span(
  name: string,
  start: number,
  end: number,
): ClientSpan | undefined {
  return start > 0 && end > start
    ? { name, start_ms: start, duration_ms: end - start }
    : undefined;
}

/**
 * The phases of a page load as spans. Phases the browser skipped are left out.
 */
export function navigationSpans(
  nav: PerformanceNavigationTiming,
): ClientSpan[] {
  const phases = [
    span("dns", nav.domainLookupStart, nav.domainLookupEnd),
    span("connect", nav.connectStart, nav.connectEnd),
    span("request", nav.requestStart, nav.responseStart),
    span("response", nav.responseStart, nav.responseEnd),
    span("dom", nav.responseEnd, nav.domContentLoadedEventEnd),
    span("load", nav.domContentLoadedEventEnd, nav.loadEventEnd),
  ];
  return phases.filter((phase): phase is ClientSpan => phase !== undefined);
}

/**
 * Builds the report for a page view, or nothing when the page has no server
 * trace.
 */
export function navigationReport(
  nav: PerformanceNavigationTiming,
  vitals: Vitals,
  page: string,
  timeOrigin: number,
): ClientReport | undefined {
  const traceparent = serverTraceparent(nav);
  if (traceparent === undefined) {
    return undefined;
  }
  const ttfb = nav.responseStart - nav.requestStart;
  return {
    traceparent,
    page,
    request_start_ms: nav.requestStart,
    time_origin_ms: timeOrigin,
    spans: navigationSpans(nav).map((s) => ({
      ...s,
      attributes: { type: nav.type },
    })),
    vitals: ttfb > 0 ? { ...vitals, ttfb_ms: ttfb } : vitals,
  };
}

/**
 * Builds the report for a slow API call, or nothing when it was fast or
 * untraced.
 */
export function resourceReport(
  entry: PerformanceResourceTiming,
  page: string,
  timeOrigin: number,
): ClientReport | undefined {
  const traceparent = serverTraceparent(entry);
  if (traceparent === undefined || entry.duration < SLOW_FETCH_MS) {
    return undefined;
  }
  const path = new URL(entry.name, globalThis.location.href).pathname;
  return {
    traceparent,
    page,
    request_start_ms:
      entry.requestStart > 0 ? entry.requestStart : entry.startTime,
    time_origin_ms: timeOrigin,
    spans: [
      {
        name: `fetch ${path}`,
        start_ms: entry.startTime,
        duration_ms: entry.duration,
        attributes: { initiator: entry.initiatorType },
      },
    ],
    vitals: {},
  };
}

interface ShiftEntry extends PerformanceEntry {
  readonly value?: number;
  readonly hadRecentInput?: boolean;
  readonly interactionId?: number;
}

/** The observer constructor; injectable because jsdom has none. */
type ObserverConstructor = new (callback: PerformanceObserverCallback) => {
  observe(options: PerformanceObserverInit): void;
  disconnect(): void;
};

function availableObserver(): ObserverConstructor | undefined {
  return (globalThis as { PerformanceObserver?: ObserverConstructor })
    .PerformanceObserver;
}

/** Collects Core Web Vitals from `PerformanceObserver` entries. */
export class VitalsCollector {
  private readonly vitals: Vitals = {};
  private readonly observers: { disconnect(): void }[] = [];

  /** Starts observing; entry types the browser lacks are skipped. */
  observe(
    Observer: ObserverConstructor | undefined = availableObserver(),
  ): this {
    if (Observer === undefined) {
      return this;
    }
    for (const type of [
      "largest-contentful-paint",
      "layout-shift",
      "event",
      "paint",
    ]) {
      try {
        const observer = new Observer((list) => {
          for (const entry of list.getEntries()) {
            this.add(entry);
          }
        });
        observer.observe({ type, buffered: true });
        this.observers.push(observer);
      } catch {
        // Unsupported entry type in this browser.
      }
    }
    return this;
  }

  /** Feeds one entry; exposed for tests. */
  add(entry: ShiftEntry): void {
    switch (entry.entryType) {
      case "largest-contentful-paint": {
        this.vitals.lcp_ms = entry.startTime;
        break;
      }
      case "layout-shift": {
        if (entry.hadRecentInput !== true) {
          this.vitals.cls = (this.vitals.cls ?? 0) + (entry.value ?? 0);
        }
        break;
      }
      case "event": {
        if ((entry.interactionId ?? 0) > 0) {
          this.vitals.inp_ms = Math.max(
            this.vitals.inp_ms ?? 0,
            entry.duration,
          );
        }
        break;
      }
      case "paint": {
        if (entry.name === "first-contentful-paint") {
          this.vitals.fcp_ms = entry.startTime;
        }
        break;
      }
      default: {
        break;
      }
    }
  }

  /** The vitals measured so far. */
  snapshot(): Vitals {
    return { ...this.vitals };
  }

  /** Stops observing. */
  disconnect(): void {
    for (const observer of this.observers.splice(0)) {
      observer.disconnect();
    }
  }
}

/** Posts a report. Resolves to whether the server accepted it; never throws. */
export async function sendReport(
  report: ClientReport,
  doFetch: typeof fetch = globalThis.fetch.bind(globalThis),
  baseUrl = "",
): Promise<boolean> {
  try {
    const response = await doFetch(`${baseUrl}${CLIENT_TELEMETRY_PATH}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(report),
      credentials: "include",
      keepalive: true,
    });
    return response.ok;
  } catch {
    return false;
  }
}
