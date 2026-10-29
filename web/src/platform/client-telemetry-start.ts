// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  navigationReport,
  resourceReport,
  sendReport,
  VitalsCollector,
  type ClientReport,
} from "./client-telemetry";

/** What {@link startClientTelemetry} needs from the browser; injectable for tests. */
export interface TelemetryEnvironment {
  readonly performance: Pick<Performance, "getEntriesByType" | "timeOrigin">;
  readonly location: Pick<Location, "pathname">;
  readonly document: Pick<Document, "visibilityState" | "addEventListener" | "removeEventListener">;
  readonly Observer?: typeof PerformanceObserver | undefined;
  readonly send?: (report: ClientReport) => Promise<boolean>;
}

/**
 * Starts reporting: slow API calls as they finish, and the page view with
 * its vitals once the page is hidden. Returns a function that stops it.
 */
export function startClientTelemetry(environment: TelemetryEnvironment): () => void {
  const send = environment.send ?? ((report: ClientReport) => sendReport(report));
  const origin = environment.performance.timeOrigin;
  const vitals = new VitalsCollector().observe(environment.Observer);
  let navigationSent = false;

  const resources = environment.Observer
    ? new environment.Observer((list) => {
        for (const entry of list.getEntries()) {
          const report = resourceReport(
            entry as PerformanceResourceTiming,
            environment.location.pathname,
            origin,
          );
          if (report !== undefined) {
            void send(report);
          }
        }
      })
    : undefined;
  resources?.observe({ type: "resource", buffered: false });

  const onHidden = () => {
    if (navigationSent || environment.document.visibilityState !== "hidden") {
      return;
    }
    const [nav] = environment.performance.getEntriesByType("navigation");
    if (nav === undefined) {
      return;
    }
    const report = navigationReport(
      nav as PerformanceNavigationTiming,
      vitals.snapshot(),
      environment.location.pathname,
      origin,
    );
    if (report !== undefined) {
      navigationSent = true;
      void send(report);
    }
  };
  environment.document.addEventListener("visibilitychange", onHidden);

  return () => {
    environment.document.removeEventListener("visibilitychange", onHidden);
    resources?.disconnect();
    vitals.disconnect();
  };
}
