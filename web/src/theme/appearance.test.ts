// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import {
  applyAppearance,
  DEFAULT_APPEARANCE,
  effectiveSeed,
  loadAppearance,
  resolveTheme,
  saveAppearance,
  type Appearance,
} from "./appearance";
import { THEME_FORMAT } from "./theme-file";
import { buildScheme, DEFAULT_SEED } from "./themes";

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
    const saved = JSON.stringify({
      theme: "sepia",
      seed: "#ABCDEF",
      density: "spacious",
      motion: "reduce",
      textSize: 125,
      font: "dyslexic",
    });
    expect(loadAppearance(storing(saved))).toEqual({
      theme: "sepia",
      seed: "#abcdef",
      density: "spacious",
      motion: "reduce",
      textSize: 125,
      font: "dyslexic",
      custom: undefined,
    });
  });

  it("replaces each invalid field with its default", () => {
    const saved = JSON.stringify({
      theme: "neon",
      seed: "red",
      density: "roomy",
      motion: "wild",
      textSize: 300,
      font: "papyrus",
      custom: { format: "nope" },
    });
    expect(loadAppearance(storing(saved))).toEqual(DEFAULT_APPEARANCE);
    expect(loadAppearance(storing(JSON.stringify({ seed: 42 })))).toEqual(DEFAULT_APPEARANCE);
  });

  it("treats Vellum's own seed, saved by older versions, as following the organisation", () => {
    const saved = JSON.stringify({ theme: "dark", seed: "#4F5B92", density: "compact" });
    expect(loadAppearance(storing(saved)).seed).toBeUndefined();
  });

  it("keeps a valid custom theme and drops one that fails contrast", () => {
    const custom = {
      format: THEME_FORMAT,
      name: "Ink",
      base: "light",
      seed: "#225588",
      tokens: {},
    };
    const good = loadAppearance(storing(JSON.stringify({ theme: "custom", custom })));
    expect(good.theme).toBe("custom");
    expect(good.custom?.name).toBe("Ink");
    const unreadable = { ...custom, tokens: { "on-surface": "#fafafa" } };
    const bad = loadAppearance(storing(JSON.stringify({ theme: "custom", custom: unreadable })));
    expect(bad.custom).toBeUndefined();
    expect(bad.theme).toBe("system");
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
    const value: Appearance = { ...DEFAULT_APPEARANCE, theme: "dim", seed: "#112233" };
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

  it("uses a custom theme's start, or the OS without one", () => {
    const custom = { name: "Ink", base: "sepia", seed: "#225588", tokens: {} } as const;
    expect(resolveTheme("custom", true, custom)).toBe("sepia");
    expect(resolveTheme("custom", true)).toBe("dark");
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

  it("writes the reading settings and the text size", () => {
    const root = document.createElement("div");
    const value = {
      ...DEFAULT_APPEARANCE,
      motion: "reduce",
      font: "dyslexic",
      textSize: 125,
    } as const;
    applyAppearance(root, value, false);
    expect(root.dataset["motion"]).toBe("reduce");
    expect(root.dataset["font"]).toBe("dyslexic");
    expect(root.style.fontSize).toBe("125%");
    applyAppearance(root, DEFAULT_APPEARANCE, false);
    expect(root.style.fontSize).toBe("");
  });

  it("follows the organisation's seed until the user picks one", () => {
    const root = document.createElement("div");
    applyAppearance(root, DEFAULT_APPEARANCE, false, "#b02020");
    expect(root.style.getPropertyValue("--md-primary")).toBe(
      buildScheme("light", "#b02020")["primary"],
    );
    applyAppearance(root, { ...DEFAULT_APPEARANCE, seed: "#205020" }, false, "#b02020");
    expect(root.style.getPropertyValue("--md-primary")).toBe(
      buildScheme("light", "#205020")["primary"],
    );
    expect(effectiveSeed(DEFAULT_APPEARANCE)).toBe(DEFAULT_SEED);
  });

  it("applies a custom theme's tokens and marks the page", () => {
    const root = document.createElement("div");
    const custom = {
      name: "Ink",
      base: "light",
      seed: "#225588",
      tokens: { primary: "#123456" },
    } as const;
    applyAppearance(root, { ...DEFAULT_APPEARANCE, theme: "custom", custom }, true);
    expect(root.style.getPropertyValue("--md-primary")).toBe("#123456");
    expect(root.dataset["theme"]).toBe("light");
    expect(root.dataset["customTheme"]).toBe("Ink");
    applyAppearance(root, DEFAULT_APPEARANCE, false);
    expect(root.dataset["customTheme"]).toBeUndefined();
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
