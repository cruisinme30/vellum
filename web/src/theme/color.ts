// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Colour maths for Material 3 tonal palettes.
 *
 * A tone is CIE L* (0 black, 100 white), as in Material 3, so the contrast
 * between two tones is the same whatever the seed hue. Colours are built
 * in CIE LCh and pulled into the sRGB gamut by lowering chroma.
 */

/** An sRGB colour with channels from 0 to 1. */
export interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

/** A colour in CIE LCh(ab), D65. */
export interface Lch {
  readonly l: number;
  readonly c: number;
  readonly h: number;
}

const WHITE = { x: 0.950_47, y: 1, z: 1.088_83 };
const EPSILON = 216 / 24_389;
const KAPPA = 24_389 / 27;

/** Parses `#rgb` or `#rrggbb`. */
export function parseHex(hex: string): Rgb | undefined {
  const match = /^#?([\da-f]{3}|[\da-f]{6})$/i.exec(hex.trim());
  const body = match?.[1];
  if (body === undefined) {
    return undefined;
  }
  const full =
    body.length === 3 ? body.replaceAll(/[\da-f]/gi, (c) => c + c) : body;
  const value = Number.parseInt(full, 16);
  return {
    r: ((value >> 16) & 255) / 255,
    g: ((value >> 8) & 255) / 255,
    b: (value & 255) / 255,
  };
}

const channel = (v: number) =>
  Math.round(Math.min(1, Math.max(0, v)) * 255)
    .toString(16)
    .padStart(2, "0");

/** Formats a colour as lowercase `#rrggbb`. */
export function toHex({ r, g, b }: Rgb): string {
  return `#${channel(r)}${channel(g)}${channel(b)}`;
}

const toLinear = (v: number) =>
  v <= 0.040_45 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
const fromLinear = (v: number) =>
  v <= 0.003_130_8 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055;

/** WCAG relative luminance. */
export function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

/** WCAG contrast ratio between two colours, from 1 to 21. */
export function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].toSorted((x, y) => y - x) as [
    number,
    number,
  ];
  return (hi + 0.05) / (lo + 0.05);
}

/** Converts sRGB to CIE LCh. */
export function toLch(rgb: Rgb): Lch {
  const r = toLinear(rgb.r);
  const g = toLinear(rgb.g);
  const b = toLinear(rgb.b);
  const x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / WHITE.x;
  const y = (0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b) / WHITE.y;
  const z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / WHITE.z;
  const f = (t: number) =>
    t > EPSILON ? Math.cbrt(t) : (KAPPA * t + 16) / 116;
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  const l = 116 * fy - 16;
  const a = 500 * (fx - fy);
  const bb = 200 * (fy - fz);
  const h = (Math.atan2(bb, a) * 180) / Math.PI;
  return { l, c: Math.hypot(a, bb), h: h < 0 ? h + 360 : h };
}

function lchToRaw({ l, c, h }: Lch): { rgb: Rgb; inGamut: boolean } {
  const a = c * Math.cos((h * Math.PI) / 180);
  const b = c * Math.sin((h * Math.PI) / 180);
  const fy = (l + 16) / 116;
  const fx = fy + a / 500;
  const fz = fy - b / 200;
  const inv = (t: number) =>
    t ** 3 > EPSILON ? t ** 3 : (116 * t - 16) / KAPPA;
  const x = inv(fx) * WHITE.x;
  const y = (l > KAPPA * EPSILON ? fy ** 3 : l / KAPPA) * WHITE.y;
  const z = inv(fz) * WHITE.z;
  const lr = 3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z;
  const lg = -0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z;
  const lb = 0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z;
  const inGamut = [lr, lg, lb].every((v) => v >= -0.0001 && v <= 1.0001);
  return {
    rgb: { r: fromLinear(lr), g: fromLinear(lg), b: fromLinear(lb) },
    inGamut,
  };
}

/**
 * Converts CIE LCh to sRGB, lowering chroma until the colour fits the gamut.
 */
export function fromLch(lch: Lch): Rgb {
  let low = 0;
  let high = lch.c;
  let best = lchToRaw({ ...lch, c: 0 }).rgb;
  for (let step = 0; step < 24; step += 1) {
    const mid = (low + high) / 2;
    const attempt = lchToRaw({ ...lch, c: mid });
    if (attempt.inGamut) {
      best = attempt.rgb;
      low = mid;
    } else {
      high = mid;
    }
  }
  const exact = lchToRaw(lch);
  return exact.inGamut ? exact.rgb : best;
}

/** A tonal palette: one hue and chroma, any tone. */
export function tone(seed: Lch, chroma: number, value: number): string {
  return toHex(fromLch({ l: value, c: chroma, h: seed.h }));
}
