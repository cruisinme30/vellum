// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useSyncExternalStore } from "react";

/**
 * Whether the server is answering in full (issue #191, ADR 0061 D267).
 *
 * The HTTP client reports every answer: a 503 means part of Vellum is down
 * (the bucket, most often) and the server is serving what it still can; the
 * next successful answer means it is back. The shell shows a banner while
 * it is degraded. Network failures are not reported here: being offline is
 * the browser's to say.
 */
export interface ServiceHealth {
  readonly degraded: boolean;
  /** When the server asked us to try again (ms since the epoch), if it did. */
  readonly retryAt: number | undefined;
}

const HEALTHY: ServiceHealth = { degraded: false, retryAt: undefined };

/** A small observable holding the latest {@link ServiceHealth}. */
export class HealthMonitor {
  #health: ServiceHealth = HEALTHY;
  readonly #listeners = new Set<() => void>();

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  readonly snapshot = (): ServiceHealth => this.#health;

  /** Records one answer's HTTP status and `Retry-After` (seconds). */
  report(
    status: number,
    retryAfterSeconds?: number,
    now: number = Date.now(),
  ): void {
    let next: ServiceHealth | undefined;
    if (status === 503) {
      next = {
        degraded: true,
        retryAt:
          retryAfterSeconds === undefined
            ? undefined
            : now + retryAfterSeconds * 1000,
      };
    } else if (status < 500 && this.#health.degraded) {
      next = HEALTHY;
    }
    if (next === undefined) return;
    if (
      next.degraded === this.#health.degraded &&
      next.retryAt === this.#health.retryAt
    )
      return;
    this.#health = next;
    for (const listener of this.#listeners) listener();
  }
}

/** The app's one monitor, fed by the HTTP client. */
export const serviceHealth = new HealthMonitor();

/** The current {@link ServiceHealth}, re-rendering when it changes. */
export function useServiceHealth(
  monitor: HealthMonitor = serviceHealth,
): ServiceHealth {
  return useSyncExternalStore(monitor.subscribe, monitor.snapshot);
}

/** `Retry-After` in seconds, when it is a number (dates are ignored). */
export function retryAfterSeconds(header: string | null): number | undefined {
  if (header === null) return undefined;
  const seconds = Number(header.trim());
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : undefined;
}
