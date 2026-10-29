// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { startUpdates, type Updater } from "./updater.js";

function fakeUpdater(result: () => Promise<unknown>) {
  const check = vi.fn(result);
  const listeners = new Map<string, (value: never) => void>();
  const updater = {
    autoDownload: false,
    autoInstallOnAppQuit: false,
    checkForUpdates: check,
    on(event: string, listener: (value: never) => void) {
      listeners.set(event, listener);
      return updater;
    },
  };
  return {
    check,
    updater: updater as Updater & typeof updater,
    emit: (event: string, value: unknown) => listeners.get(event)?.(value as never),
  };
}

describe("startUpdates", () => {
  it("checks now and on an interval, and reports a downloaded update", () => {
    const { updater, emit, check } = fakeUpdater(() => Promise.resolve());
    const onReady = vi.fn<(version: string) => void>();
    const timers: [() => void, number][] = [];
    startUpdates(updater, {
      onReady,
      onError: vi.fn<(error: Error) => void>(),
      setInterval: (callback, ms) => timers.push([callback, ms]),
    });
    expect(updater.autoDownload).toBe(true);
    expect(updater.autoInstallOnAppQuit).toBe(true);
    expect(check).toHaveBeenCalledTimes(1);
    expect(timers[0]?.[1]).toBe(4 * 60 * 60_000);
    timers[0]?.[0]();
    expect(check).toHaveBeenCalledTimes(2);
    emit("update-downloaded", { version: "1.0.1" });
    expect(onReady).toHaveBeenCalledWith("1.0.1");
  });

  it("reports failed checks", async () => {
    const failure = new Error("offline");
    const { updater } = fakeUpdater(() => Promise.reject(failure));
    const onError = vi.fn<(error: Error) => void>();
    startUpdates(updater, {
      onReady: vi.fn<(version: string) => void>(),
      onError,
      setInterval: () => 0,
      intervalMs: 5,
    });
    await vi.waitFor(() => {
      expect(onError).toHaveBeenCalledWith(failure);
    });
  });
});
