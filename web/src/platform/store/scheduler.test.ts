// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SyncScheduler } from "./scheduler";
import { SyncError } from "./transport";

// Jitter of 1 makes the backoff exactly its base value.
const one = () => 0.999_999_999;

describe("SyncScheduler", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("debounces local changes into one sync", async () => {
    const sync = vi.fn(() => Promise.resolve());
    const scheduler = new SyncScheduler(
      { sync },
      { debounceMs: 50, pollMs: 1000, random: one },
    );
    scheduler.changed();
    scheduler.changed();
    await vi.advanceTimersByTimeAsync(49);
    expect(sync).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(sync).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(sync).toHaveBeenCalledTimes(2);
    scheduler.stop();
  });

  it("backs off exponentially on retryable failures and resets on success", async () => {
    let fail = 3;
    const sync = vi.fn(() => {
      if (fail > 0) {
        fail -= 1;
        return Promise.reject(new SyncError("down", 503, true, "r"));
      }
      return Promise.resolve();
    });
    const scheduler = new SyncScheduler(
      { sync },
      { pollMs: 100_000, maxBackoffMs: 3000, random: one },
    );
    scheduler.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(scheduler.failures).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(scheduler.failures).toBe(2);
    scheduler.changed(); // a change during backoff waits for the backoff
    await vi.advanceTimersByTimeAsync(1999);
    expect(sync).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(scheduler.failures).toBe(3);
    await vi.advanceTimersByTimeAsync(3000); // capped at maxBackoffMs
    expect(scheduler.failures).toBe(0);
    expect(sync).toHaveBeenCalledTimes(4);
    scheduler.stop();
  });

  it("stops on a fatal error and reports it", async () => {
    const fatal = new SyncError("forbidden", 403, false, "r");
    const sync = vi.fn(() => Promise.reject(fatal));
    const onFatal = vi.fn();
    const scheduler = new SyncScheduler({ sync }, { onFatal });
    scheduler.start();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(sync).toHaveBeenCalledTimes(1);
    expect(onFatal).toHaveBeenCalledWith(fatal);
  });

  it("treats unknown errors as retryable", async () => {
    const sync = vi.fn(() => Promise.reject(new Error("boom")));
    const scheduler = new SyncScheduler({ sync });
    scheduler.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(scheduler.failures).toBe(1);
    scheduler.stop();
  });

  it("pauses while offline and syncs at once when back online", async () => {
    const sync = vi.fn(() => Promise.resolve());
    const scheduler = new SyncScheduler({ sync });
    scheduler.setOnline(false);
    scheduler.changed();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sync).not.toHaveBeenCalled();
    scheduler.setOnline(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(sync).toHaveBeenCalledTimes(1);
    scheduler.stop();
    scheduler.changed();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sync).toHaveBeenCalledTimes(1);
  });

  it("uses real timers by default", () => {
    const scheduler = new SyncScheduler({ sync: () => Promise.resolve() });
    scheduler.changed();
    expect(vi.getTimerCount()).toBe(1);
    scheduler.stop();
    expect(vi.getTimerCount()).toBe(0);
  });
});
