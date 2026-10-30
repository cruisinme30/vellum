// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { contrast, fromLch, parseHex, toHex, toLch } from "./color";
import {
  buildScheme,
  checkContrast,
  CONTRAST_RULES,
  DEFAULT_SEED,
  isDark,
  THEMES,
  type ResolvedTheme,
} from "./themes";

const resolved = THEMES.filter((t): t is ResolvedTheme => t !== "system");

describe("colour maths", () => {
  it("parses and formats hex", () => {
    expect(parseHex("#fff")).toEqual({ r: 1, g: 1, b: 1 });
    expect(toHex(parseHex("#4F5B92") ?? { r: 0, g: 0, b: 0 })).toBe("#4f5b92");
    expect(parseHex("blue")).toBeUndefined();
  });

  it("round-trips through LCh", () => {
    for (const hex of ["#4f5b92", "#dde1ff", "#1d1b20", "#c0ffee"]) {
      const rgb = parseHex(hex);
      expect(rgb).toBeDefined();
      if (rgb) {
        expect(toHex(fromLch(toLch(rgb)))).toBe(hex);
      }
    }
  });

  it("clamps out-of-gamut chroma instead of failing", () => {
    expect(toHex(fromLch({ l: 50, c: 400, h: 140 }))).toMatch(/^#[\da-f]{6}$/);
  });

  it("measures WCAG contrast", () => {
    const black = { r: 0, g: 0, b: 0 };
    const white = { r: 1, g: 1, b: 1 };
    expect(contrast(black, white)).toBeCloseTo(21, 5);
    expect(contrast(white, white)).toBe(1);
  });
});

describe("themes", () => {
  it("uses the brief's light values for the default seed", () => {
    const light = buildScheme("light");
    expect(light["primary"]).toBe("#4f5b92");
    expect(light["primary-container"]).toBe("#dde1ff");
    expect(light["surface"]).toBe("#f7f8fc");
  });

  it.each(resolved)(
    "%s meets WCAG 2.2 AA contrast with the default seed",
    (theme) => {
      expect(checkContrast(buildScheme(theme))).toEqual([]);
    },
  );

  it.each(["#b3261e", "#00897b", "#f9a825", "#6750a4", "#000000"])(
    "every theme stays accessible with seed %s",
    (seed) => {
      for (const theme of resolved) {
        expect(
          checkContrast(buildScheme(theme, seed)),
          `${theme} ${seed}`,
        ).toEqual([]);
      }
    },
  );

  it("falls back to the default seed when the seed is not a colour", () => {
    expect(buildScheme("dark", "nope")).toEqual(
      buildScheme("dark", DEFAULT_SEED),
    );
  });

  it("reports failing pairs", () => {
    const broken = { ...buildScheme("light"), "on-surface": "#f7f8fc" };
    expect(checkContrast(broken).map((f) => f.fg)).toContain("on-surface");
  });

  it("reports every rule of a scheme with missing tokens as failing", () => {
    const failures = checkContrast({});
    expect(failures).toHaveLength(CONTRAST_RULES.length);
    expect(failures.every((failure) => failure.ratio === 0)).toBe(true);
  });

  it("knows which themes are dark", () => {
    expect(resolved.filter((theme) => isDark(theme))).toEqual(["dark", "dim"]);
  });
});
