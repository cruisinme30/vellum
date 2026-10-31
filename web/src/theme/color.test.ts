// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  contrast,
  fromLch,
  luminance,
  parseHex,
  toHex,
  toLch,
  tone,
} from "./color";

const BLACK = { r: 0, g: 0, b: 0 };
const WHITE = { r: 1, g: 1, b: 1 };

describe("parseHex", () => {
  it("accepts short and long forms, with or without #, in any case", () => {
    expect(parseHex("#f00")).toEqual({ r: 1, g: 0, b: 0 });
    expect(parseHex("00FF00")).toEqual({ r: 0, g: 1, b: 0 });
    expect(parseHex("  #0000ff  ")).toEqual({ r: 0, g: 0, b: 1 });
  });

  it("rejects anything else", () => {
    for (const value of [
      "",
      "#",
      "#ff",
      "#ffff",
      "#ggg",
      "#1234567",
      "rgb(0,0,0)",
    ]) {
      expect(parseHex(value), value).toBeUndefined();
    }
  });
});

describe("toHex", () => {
  it("clamps channels outside 0..1", () => {
    expect(toHex({ r: -0.5, g: 2, b: 0.5 })).toBe("#00ff80");
  });
});

describe("luminance and contrast", () => {
  it("spans 0 for black to 1 for white", () => {
    expect(luminance(BLACK)).toBe(0);
    expect(luminance(WHITE)).toBeCloseTo(1, 10);
    expect(luminance({ r: 0.02, g: 0.02, b: 0.02 })).toBeCloseTo(
      0.02 / 12.92,
      10,
    );
  });

  it("is symmetric", () => {
    const grey = parseHex("#777777") ?? BLACK;
    expect(contrast(grey, WHITE)).toBeCloseTo(contrast(WHITE, grey), 10);
    expect(contrast(grey, WHITE)).toBeCloseTo(4.48, 2);
  });
});

describe("LCh", () => {
  it("maps black, white and grey to zero chroma", () => {
    expect(toLch(BLACK).l).toBeCloseTo(0, 5);
    expect(toLch(WHITE).l).toBeCloseTo(100, 3);
    expect(toLch({ r: 0.5, g: 0.5, b: 0.5 }).c).toBeCloseTo(0, 3);
  });

  it("keeps hue in 0..360", () => {
    for (const hex of ["#ff0000", "#0000ff", "#ff00ff", "#00ffff"]) {
      const { h } = toLch(parseHex(hex) ?? BLACK);
      expect(h, hex).toBeGreaterThanOrEqual(0);
      expect(h, hex).toBeLessThan(360);
    }
  });

  it("converts very dark colours on the linear segment", () => {
    const dark = { r: 0.001, g: 0.001, b: 0.001 };
    expect(toHex(fromLch(toLch(dark)))).toBe(toHex(dark));
  });

  it("lowers chroma for colours outside the gamut, keeping lightness", () => {
    const vivid = fromLch({ l: 60, c: 200, h: 300 });
    expect(toLch(vivid).l).toBeCloseTo(60, 0);
    expect(toLch(vivid).c).toBeLessThan(200);
  });
});

describe("tone", () => {
  it("gives darker colours for lower tones of one hue", () => {
    const seed = toLch(parseHex("#4f5b92") ?? BLACK);
    const light = parseHex(tone(seed, 30, 90)) ?? BLACK;
    const dark = parseHex(tone(seed, 30, 20)) ?? WHITE;
    expect(luminance(light)).toBeGreaterThan(luminance(dark));
    expect(tone(seed, 0, 100)).toBe("#ffffff");
    expect(tone(seed, 0, 0)).toBe("#000000");
  });
});
