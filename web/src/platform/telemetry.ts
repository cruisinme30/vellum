// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Client telemetry hooks for the platform layer. Values are reported to a
 * sink the app installs (the OpenTelemetry web exporter once it lands);
 * until then they go to `performance.measure` so they show in DevTools.
 */

/** Attributes on a measurement. Keep them low-cardinality. */
type Attributes = Readonly<Record<string, string | number | boolean>>;

/** Where measurements go. */
export interface TelemetrySink {
  /**
   * Records one value of a histogram, in milliseconds unless the name says
   * otherwise.
   */
  histogram(name: string, value: number, attributes?: Attributes): void;
  /** Adds to a counter. */
  count(name: string, attributes?: Attributes): void;
}

const performanceSink: TelemetrySink = {
  histogram(name, value) {
    const end = performance.now();
    performance.measure(name, { start: end - value, end });
  },
  count() {
    // Counters have no DevTools equivalent; the real sink exports them.
  },
};

let sink: TelemetrySink = performanceSink;

/** Installs the sink. Returns the previous one, so tests can restore it. */
export function setTelemetrySink(next: TelemetrySink): TelemetrySink {
  const previous = sink;
  sink = next;
  return previous;
}

/** The current sink. */
export function telemetry(): TelemetrySink {
  return sink;
}

function hex(bytes: number): string {
  const buffer = new Uint8Array(bytes);
  crypto.getRandomValues(buffer);
  return [...buffer].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** A fresh request ID for `X-Request-Id`. */
export function newRequestId(): string {
  return crypto.randomUUID();
}

/** A fresh W3C `traceparent` (version 00, sampled) for a root client span. */
export function newTraceparent(): string {
  return `00-${hex(16)}-${hex(8)}-01`;
}
