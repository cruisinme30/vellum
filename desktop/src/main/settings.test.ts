// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { memoryDisk } from "../test-support/memory-disk.js";

import {
  DEFAULT_SETTINGS,
  loadSettings,
  normalizeServerUrl,
  saveSettings,
  settingsFromArgv,
} from "./settings.js";

describe("settings", () => {
  it("normalises server addresses to an https origin", () => {
    expect(normalizeServerUrl("vellum.northstar.dev/some/path")).toBe(
      "https://vellum.northstar.dev",
    );
    expect(normalizeServerUrl("http://127.0.0.1:7700/")).toBe(
      "http://127.0.0.1:7700",
    );
    // eslint-disable-next-line sonarjs/no-clear-text-protocols -- checks that plain HTTP is refused
    expect(() => normalizeServerUrl("http://vellum.northstar.dev")).toThrow(
      /https/,
    );
    expect(() =>
      normalizeServerUrl("https://a:b@vellum.northstar.dev"),
    ).toThrow(/password/);
  });

  it("defaults to local mode", async () => {
    expect(await loadSettings(memoryDisk())).toEqual(DEFAULT_SETTINGS);
    expect(await loadSettings(memoryDisk("{not json"))).toEqual(
      DEFAULT_SETTINGS,
    );
    expect(await loadSettings(memoryDisk('{"mode":"remote"}'))).toEqual(
      DEFAULT_SETTINGS,
    );
    expect(await loadSettings(memoryDisk("[]"))).toEqual(DEFAULT_SETTINGS);
  });

  it("round-trips a remote server", async () => {
    const disk = memoryDisk();
    await saveSettings(disk, {
      mode: "remote",
      serverUrl: "https://vellum.northstar.dev",
    });
    expect(await loadSettings(disk)).toEqual({
      mode: "remote",
      serverUrl: "https://vellum.northstar.dev",
    });
  });

  it("reads the command line", () => {
    expect(settingsFromArgv(["--local"])).toEqual({ mode: "local" });
    expect(settingsFromArgv(["--server=vellum.northstar.dev"])).toEqual({
      mode: "remote",
      serverUrl: "https://vellum.northstar.dev",
    });
    expect(settingsFromArgv([])).toBeUndefined();
  });
});
