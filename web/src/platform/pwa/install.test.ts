// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { captureInstallPrompt, isStandalone } from "./install";

function offer(outcome: "accepted" | "dismissed") {
  const event = new Event("beforeinstallprompt", { cancelable: true });
  return Object.assign(event, {
    prompt: vi.fn(() => Promise.resolve()),
    userChoice: Promise.resolve({ outcome }),
  });
}

const media = (matches: boolean) => () => ({ matches }) as MediaQueryList;

describe("install prompt", () => {
  it("holds the browser's offer until the app asks", async () => {
    const target = new EventTarget();
    const onAvailable = vi.fn();
    const installer = captureInstallPrompt(target, onAvailable);
    expect(installer.available).toBe(false);
    expect(await installer.prompt()).toBe(false);
    const event = offer("accepted");
    target.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(onAvailable).toHaveBeenCalled();
    expect(installer.available).toBe(true);
    expect(await installer.prompt()).toBe(true);
    expect(event.prompt).toHaveBeenCalled();
    expect(installer.available).toBe(false);
  });

  it("forgets the offer once installed and reports dismissals", async () => {
    const target = new EventTarget();
    const installer = captureInstallPrompt(target);
    target.dispatchEvent(offer("dismissed"));
    target.dispatchEvent(new Event("appinstalled"));
    expect(installer.available).toBe(false);
    target.dispatchEvent(offer("dismissed"));
    expect(await installer.prompt()).toBe(false);
  });

  it("detects standalone mode", () => {
    expect(isStandalone({ matchMedia: media(true) })).toBe(true);
    expect(isStandalone({ matchMedia: media(false) })).toBe(false);
    expect(isStandalone({})).toBe(false);
  });
});
