// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";

import {
  channelsFor,
  createHttpNotificationPreferencesStore,
  createMockNotificationPreferencesStore,
  DEFAULT_PREFERENCES,
  defaultChannels,
  formatMinutes,
  fromWire,
  isUrgent,
  KINDS,
  parseMinutes,
  PREFERENCES_PATH,
  toWire,
  withChannel,
  type PreferencesWire,
} from "./preferences";

// The server sends null for "not snoozed"; the lint rules forbid the literal.
const NULL = JSON.parse("null") as null;

describe("defaults", () => {
  it("match vellum_notify's default channels", () => {
    expect(defaultChannels("mentioned")).toEqual(["in_app", "email", "push"]);
    expect(defaultChannels("triage_new")).toEqual(["in_app", "push"]);
    expect(defaultChannels("commented")).toEqual(["in_app", "email"]);
    expect(defaultChannels("pull_request")).toEqual(["in_app"]);
    expect(KINDS).toHaveLength(11);
    expect(KINDS.filter((kind) => isUrgent(kind))).toEqual(["sla_breached", "reminder"]);
  });
});

describe("withChannel", () => {
  it("stores only kinds that differ from their defaults", () => {
    const off = withChannel(DEFAULT_PREFERENCES, "mentioned", "email", false);
    expect(off.channels).toEqual({ mentioned: ["in_app", "push"] });
    expect(channelsFor(off, "mentioned")).toEqual(["in_app", "push"]);
    // Turning it back on returns to the defaults, so the kind leaves the map.
    expect(withChannel(off, "mentioned", "email", true).channels).toEqual({});
  });

  it("keeps channels in table order and leaves other kinds alone", () => {
    const start = { ...DEFAULT_PREFERENCES, channels: { commented: ["push"] as const } };
    const next = withChannel(start, "state_changed", "push", true);
    expect(next.channels).toEqual({ commented: ["push"], state_changed: ["in_app", "push"] });
    expect(withChannel(next, "state_changed", "in_app", false).channels.state_changed).toEqual([
      "push",
    ]);
  });
});

describe("durations", () => {
  it.each([
    ["PT10M", 10],
    ["PT1H30M", 90],
    ["PT0S", 0],
    ["PT90S", 1],
    ["10m", 10],
    ["1h 30m", 90],
    ["2 hours", 120],
    ["", 10],
    ["PT", 10],
    ["soon", 10],
  ])("reads %j as %i minutes", (text, minutes) => {
    expect(parseMinutes(text)).toBe(minutes);
  });

  it("writes ISO 8601", () => {
    expect(formatMinutes(0)).toBe("PT0S");
    expect(formatMinutes(30)).toBe("PT30M");
  });
});

describe("wire form", () => {
  const wire: PreferencesWire = {
    channels: { mentioned: ["push", "in_app", "carrier_pigeon"], future_kind: ["email"] },
    email_delay: "PT5M",
    muted: ["ENG-142", "ENG-142"],
    snoozed_until: NULL,
  };

  it("drops what a newer server knows and this app does not", () => {
    expect(fromWire(wire)).toEqual({
      channels: { mentioned: ["in_app", "push"] },
      emailDelayMinutes: 5,
      muted: ["ENG-142"],
      snoozedUntil: undefined,
    });
  });

  it("round-trips, leaving snoozed_until out when not snoozed", () => {
    const app = fromWire(wire);
    expect(toWire(app)).toEqual({
      channels: { mentioned: ["in_app", "push"] },
      email_delay: "PT5M",
      muted: ["ENG-142"],
    });
    const snoozed = { ...app, snoozedUntil: "2026-10-30T09:00:00Z" };
    expect(toWire(snoozed).snoozed_until).toBe("2026-10-30T09:00:00Z");
    expect(fromWire(toWire(snoozed))).toEqual(snoozed);
  });
});

describe("stores", () => {
  it("reads and writes the server's preferences", async () => {
    const stored: PreferencesWire = {
      channels: {},
      email_delay: "PT10M",
      muted: [],
      snoozed_until: NULL,
    };
    const get = vi.fn(() => Promise.resolve(stored));
    const send = vi.fn((_method: string, _path: string, body: unknown) => Promise.resolve(body));
    const http = { get, send, forgetSession: vi.fn() } as unknown as Http;
    const store = createHttpNotificationPreferencesStore(http);

    expect(await store.load()).toEqual(fromWire(stored));
    expect(get).toHaveBeenCalledWith(PREFERENCES_PATH);

    const saved = await store.save({ ...DEFAULT_PREFERENCES, emailDelayMinutes: 0 });
    expect(send).toHaveBeenCalledWith("PUT", PREFERENCES_PATH, {
      channels: {},
      email_delay: "PT0S",
      muted: [],
    });
    expect(saved.emailDelayMinutes).toBe(0);
  });

  it("keeps the demo's preferences in memory", async () => {
    const store = createMockNotificationPreferencesStore();
    const demo = await store.load();
    expect(demo.muted).toEqual(["ENG-118", "ENG-142"]);
    await store.save({ ...demo, muted: [] });
    const reloaded = await store.load();
    expect(reloaded.muted).toEqual([]);
  });
});
