// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";

import { createPlatform, type DesktopBridge, desktopBridge, startPlatform } from "./platform";

function noop(): void {
  // Nothing to unsubscribe in the fake.
}

function bridge() {
  const openExternal = vi.fn(() => Promise.resolve());
  const notify = vi.fn(() => Promise.resolve());
  const desktop: DesktopBridge = {
    version: "1.0.0",
    os: "darwin",
    serverUrl: () => Promise.resolve("https://northstar.dev"),
    signIn: () => Promise.resolve({ account: "ada@northstar.dev" }),
    signOut: () => Promise.resolve(),
    accessToken: () => Promise.resolve("t"),
    openExternal,
    notify,
    onOpenUrl: () => noop,
  };
  return { desktop, openExternal, notify };
}

const notice = { title: "ENG-1", body: "Assigned", url: "/issues/ENG-1" };

describe("platform", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("uses the desktop bridge when the preload installed one", async () => {
    const { desktop, openExternal, notify } = bridge();
    expect(desktopBridge({ vellumDesktop: "nope" })).toBeUndefined();
    const platform = createPlatform({ vellumDesktop: desktop });
    expect(platform.kind).toBe("desktop");
    expect(platform.desktop).toBe(desktop);
    platform.openExternal("https://example.com");
    expect(openExternal).toHaveBeenCalledWith("https://example.com");
    expect(await platform.notify(notice)).toBe(true);
    expect(notify).toHaveBeenCalledWith(notice);
    expect(await startPlatform(platform, { production: true })).toBeUndefined();
  });

  it("is a browser tab or an installed PWA otherwise", () => {
    const installed = { matchMedia: () => ({ matches: true }) as MediaQueryList };
    expect(createPlatform(installed).kind).toBe("pwa");
    expect(createPlatform({}).kind).toBe("browser");
  });

  it("opens external links in a new tab without an opener", () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    createPlatform({}).openExternal("https://example.com");
    expect(open).toHaveBeenCalledWith("https://example.com", "_blank", "noopener,noreferrer");
  });

  it("shows web notifications only with permission and a worker", async () => {
    const platform = createPlatform({});
    expect(await platform.notify(notice)).toBe(false);
    const showNotification = vi.fn(() => Promise.resolve());
    const holder: { registration?: object } = {};
    vi.stubGlobal("Notification", { permission: "granted" });
    vi.stubGlobal("navigator", {
      serviceWorker: { getRegistration: () => Promise.resolve(holder.registration) },
    });
    expect(await platform.notify(notice)).toBe(false);
    holder.registration = { showNotification };
    expect(await platform.notify(notice)).toBe(true);
    expect(showNotification).toHaveBeenCalledWith("ENG-1", {
      body: "Assigned",
      data: "/issues/ENG-1",
    });
  });

  it("skips the worker in development and registers it in production", async () => {
    const register = vi.fn(() => Promise.reject(new Error("no")));
    const persist = vi.fn(() => Promise.resolve(true));
    vi.stubGlobal("navigator", { serviceWorker: { register }, storage: { persist } });
    const platform = createPlatform({});
    expect(await startPlatform(platform, { production: false })).toBeUndefined();
    expect(register).not.toHaveBeenCalled();
    await startPlatform(platform, { production: true, onUpdateReady: noop });
    expect(persist).toHaveBeenCalled();
    expect(register).toHaveBeenCalled();
    vi.stubGlobal("navigator", { serviceWorker: { register } });
    await startPlatform(platform, { production: true });
    expect(register).toHaveBeenCalledTimes(2);
  });

  it("registers the worker while the browser still asks to keep storage", async () => {
    const register = vi.fn(() => Promise.reject(new Error("no")));
    // Firefox shows a prompt for persist(); this one is never answered.
    const persist = vi.fn(() => new Promise<boolean>(noop));
    vi.stubGlobal("navigator", { serviceWorker: { register }, storage: { persist } });
    await startPlatform(createPlatform({}), { production: true });
    expect(persist).toHaveBeenCalled();
    expect(register).toHaveBeenCalled();
  });
});
