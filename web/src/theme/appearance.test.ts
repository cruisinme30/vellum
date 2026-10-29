// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import {
  applyAppearance,
  DEFAULT_APPEARANCE,
  loadAppearance,
  resolveTheme,
  saveAppearance,
  type Appearance,
} from "./appearance";
import { buildScheme } from "./themes";

const KEY = "vellum.appearance";

const NO_STORAGE = undefined;

/** A storage stub; like the Web Storage API it answers `null` for a missing key. */
const storing = (value?: string) => ({
  // eslint-disable-next-line unicorn/no-null -- Storage.getItem returns null for a missing key.
  getItem: () => value ?? null,
});

describe("loadAppearance", () => {
  it("returns the defaults without storage or saved settings", () => {
    expect(loadAppearance(NO_STORAGE)).toBe(DEFAULT_APPEARANCE);
    expect(loadAppearance(storing())).toBe(DEFAULT_APPEARANCE);
  });

  it("reads valid settings and lowercases the seed", () => {
    const saved = JSON.stringify({ theme: "sepia", seed: "#ABCDEF", density: "compact" });
    expect(loadAppearance(storing(saved))).toEqual({
      theme: "sepia",
      seed: "#abcdef",
      density: "compact",
    });
  });

  it("replaces each invalid field with its default", () => {
    const saved = JSON.stringify({ theme: "neon", seed: "red", density: "roomy" });
    expect(loadAppearance(storing(saved))).toEqual(DEFAULT_APPEARANCE);
    expect(loadAppearance(storing(JSON.stringify({ seed: 42 })))).toEqual(DEFAULT_APPEARANCE);
  });

  it("falls back on JSON that is not an object or does not parse", () => {
    expect(loadAppearance(storing("7"))).toBe(DEFAULT_APPEARANCE);
    expect(loadAppearance(storing("{oops"))).toBe(DEFAULT_APPEARANCE);
  });

  it("falls back when storage throws", () => {
    const blocked = {
      getItem: () => {
        throw new Error("SecurityError");
      },
    };
    expect(loadAppearance(blocked)).toBe(DEFAULT_APPEARANCE);
  });
});

describe("saveAppearance", () => {
  it("writes JSON that loadAppearance reads back", () => {
    const value: Appearance = { theme: "dim", seed: "#112233", density: "compact" };
    saveAppearance(localStorage, value);
    expect(localStorage.getItem(KEY)).toBe(JSON.stringify(value));
    expect(loadAppearance(localStorage)).toEqual(value);
  });

  it("ignores missing or refusing storage", () => {
    const refusing = {
      setItem: vi.fn(() => {
        throw new Error("QuotaExceededError");
      }),
    };
    expect(() => {
      saveAppearance(refusing, DEFAULT_APPEARANCE);
    }).not.toThrow();
    expect(refusing.setItem).toHaveBeenCalled();
    expect(() => {
      saveAppearance(NO_STORAGE, DEFAULT_APPEARANCE);
    }).not.toThrow();
  });
});

describe("resolveTheme", () => {
  it("follows the OS for system and keeps explicit themes", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("high-contrast", true)).toBe("high-contrast");
  });
});

describe("applyAppearance", () => {
  it("writes tokens, theme, density and colour scheme onto the element", () => {
    const root = document.createElement("div");
    applyAppearance(root, { ...DEFAULT_APPEARANCE, density: "compact" }, true);
    expect(root.dataset["theme"]).toBe("dark");
    expect(root.dataset["density"]).toBe("compact");
    expect(root.style.colorScheme).toBe("dark");
    expect(root.style.getPropertyValue("--md-primary")).toBe(buildScheme("dark")["primary"]);
  });

  it("uses a light colour scheme for light themes", () => {
    const root = document.createElement("div");
    applyAppearance(root, { ...DEFAULT_APPEARANCE, theme: "sepia" }, true);
    expect(root.dataset["theme"]).toBe("sepia");
    expect(root.style.colorScheme).toBe("light");
  });
});

describe("a theme preset on the page", () => {
  it("starts in that theme when nothing is saved", () => {
    expect(loadAppearance(undefined, "sepia").theme).toBe("sepia");
    expect(loadAppearance({ getItem: () => "{" }, "dim").theme).toBe("dim");
  });

  it("ignores an unknown preset and yields to saved settings", () => {
    expect(loadAppearance(undefined, "neon").theme).toBe("system");
    const saved = { getItem: () => JSON.stringify({ theme: "dark" }) };
    expect(loadAppearance(saved, "sepia").theme).toBe("dark");
  });
});
