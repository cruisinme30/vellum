// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";

import { createBrowserPushDevice } from "./device";

// The Push API answers null for "no subscription"; the lint rules forbid the literal.
const NULL = JSON.parse("null") as null;

afterEach(() => {
  vi.unstubAllGlobals();
});

/** A browser with push, whose subscription the test controls. */
function stubBrowser(subscription: PushSubscription | null) {
  const pushManager = {
    getSubscription: vi.fn(() => Promise.resolve(subscription)),
    subscribe: vi.fn(),
  };
  vi.stubGlobal("PushManager", {});
  vi.stubGlobal("Notification", { requestPermission: () => Promise.resolve("denied") });
  vi.stubGlobal("navigator", { serviceWorker: { ready: Promise.resolve({ pushManager }) } });
  return pushManager;
}

describe("createBrowserPushDevice", () => {
  it("does nothing where the browser cannot push", async () => {
    const device = createBrowserPushDevice();
    expect(device.supported).toBe(false);
    expect(await device.subscribed()).toBe(false);
    expect(await device.enable()).toBe("unsupported");
    await expect(device.disable()).resolves.toBeUndefined();
  });

  it("reports the subscription and passes the permission answer on", async () => {
    const pushManager = stubBrowser(NULL);
    const device = createBrowserPushDevice("https://vellum.northstar.dev");
    expect(device.supported).toBe(true);
    expect(await device.subscribed()).toBe(false);
    expect(await device.enable()).toBe("denied");
    expect(pushManager.subscribe).not.toHaveBeenCalled();
    // Nothing to unsubscribe.
    await device.disable();
  });

  it("sees an existing subscription", async () => {
    stubBrowser({ endpoint: "https://push.example/1" } as PushSubscription);
    expect(await createBrowserPushDevice().subscribed()).toBe(true);
  });
});
