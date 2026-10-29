// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { SyncError } from "./transport";

/** Something that can sync; {@link LocalStore} in practice. */
export interface Syncable {
  sync(): Promise<unknown>;
}

/** Timer functions, injectable for tests. */
export interface Timers {
  setTimeout(callback: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

/** Options for {@link SyncScheduler}. */
export interface SchedulerOptions {
  /** Delay after a local change before pushing, to batch keystrokes. */
  readonly debounceMs?: number;
  /** Pull interval while idle and online. */
  readonly pollMs?: number;
  /** Ceiling for the retry backoff. */
  readonly maxBackoffMs?: number;
  readonly timers?: Timers;
  /** Uniform random number in [0, 1), for jitter; injectable for tests. */
  readonly random?: () => number;
  /** Told about failures that will not be retried, for a banner. */
  readonly onFatal?: (error: unknown) => void;
}

const defaultTimers: Timers = {
  setTimeout: (callback, ms) => globalThis.setTimeout(callback, ms),
  clearTimeout: (handle) => {
    globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>);
  },
};

/** A uniform number in [0, 1) from the platform CSPRNG. */
function cryptoRandom(): number {
  const [value = 0] = crypto.getRandomValues(new Uint32Array(1));
  return value / 2 ** 32;
}

/**
 * Decides when to sync: soon after a local change (debounced so typing is
 * one push), periodically while idle, at once when the device comes back
 * online, and with exponential backoff after a retryable failure.
 */
export class SyncScheduler {
  readonly #target: Syncable;
  readonly #debounceMs: number;
  readonly #pollMs: number;
  readonly #maxBackoffMs: number;
  readonly #timers: Timers;
  readonly #random: () => number;
  readonly #onFatal: ((error: unknown) => void) | undefined;
  #timer: unknown;
  #failures = 0;
  #online = true;
  #stopped = false;

  /** @param target - the store to sync */
  constructor(target: Syncable, options: SchedulerOptions = {}) {
    this.#target = target;
    this.#debounceMs = options.debounceMs ?? 50;
    this.#pollMs = options.pollMs ?? 30_000;
    this.#maxBackoffMs = options.maxBackoffMs ?? 60_000;
    this.#timers = options.timers ?? defaultTimers;
    this.#random = options.random ?? cryptoRandom;
    this.#onFatal = options.onFatal;
  }

  /** Consecutive failures since the last success. */
  get failures(): number {
    return this.#failures;
  }

  /** Call after every local mutation. */
  changed(): void {
    this.#schedule(this.#failures === 0 ? this.#debounceMs : this.#backoff());
  }

  /** Call from the `online` and `offline` events. */
  setOnline(online: boolean): void {
    this.#online = online;
    if (online) {
      this.#failures = 0;
      this.#schedule(0);
    } else {
      this.#clear();
    }
  }

  /** Starts polling. */
  start(): void {
    this.#stopped = false;
    this.#schedule(0);
  }

  /** Stops every timer; used on sign-out and in tests. */
  stop(): void {
    this.#stopped = true;
    this.#clear();
  }

  /** The delay before the next retry: 1 s doubling, capped, with jitter. */
  #backoff(): number {
    const base = Math.min(this.#maxBackoffMs, 1000 * 2 ** Math.max(0, this.#failures - 1));
    return Math.round(base * (0.5 + this.#random() / 2));
  }

  #schedule(ms: number): void {
    if (this.#stopped || !this.#online) return;
    this.#clear();
    this.#timer = this.#timers.setTimeout(() => {
      this.#timer = undefined;
      void this.#run();
    }, ms);
  }

  #clear(): void {
    if (this.#timer !== undefined) this.#timers.clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  async #run(): Promise<void> {
    try {
      await this.#target.sync();
      this.#failures = 0;
      this.#schedule(this.#pollMs);
    } catch (error) {
      if (error instanceof SyncError && !error.retryable) {
        this.#onFatal?.(error);
        return;
      }
      this.#failures += 1;
      this.#schedule(this.#backoff());
    }
  }
}
