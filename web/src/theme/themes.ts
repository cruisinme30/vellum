// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { contrast, parseHex, toLch, tone } from "./color";

/** The built-in themes (R-TH-1). `system` follows the OS light or dark setting. */
export const THEMES = ["system", "light", "dark", "high-contrast", "sepia", "dim"] as const;
/** A built-in theme name. */
export type ThemeName = (typeof THEMES)[number];
/** A resolved theme, after `system` is replaced by light or dark. */
export type ResolvedTheme = Exclude<ThemeName, "system">;

/** The Vellum seed colour, the brief's primary. */
export const DEFAULT_SEED = "#4f5b92";

/** Colour roles, written as `--md-<role>` custom properties. */
const ROLES = [
  "primary",
  "on-primary",
  "primary-container",
  "on-primary-container",
  "secondary-container",
  "on-secondary-container",
  "surface",
  "surface-container-lowest",
  "surface-container-low",
  "surface-container",
  "surface-container-high",
  "surface-container-highest",
  "on-surface",
  "on-surface-variant",
  "outline",
  "outline-variant",
  "inverse-surface",
  "inverse-on-surface",
  "error",
  "on-error",
  "scrim",
] as const;
/** A colour role. */
type Role = (typeof ROLES)[number];

/** Callout kinds and their fixed hues, so meaning never depends on the seed. */
const CALLOUTS = { info: 255, warn: 70, good: 145, crit: 25, tip: 305, note: -1 } as const;

/** Every token of a scheme: roles plus `callout-<kind>` and `callout-<kind>-bg`. */
export type Scheme = Readonly<Record<string, string>>;

interface Tones {
  readonly dark: boolean;
  readonly roles: Readonly<
    Record<Role, readonly [palette: "p" | "s" | "n" | "nv" | "e", tone: number]>
  >;
  readonly calloutFg: number;
  readonly calloutBg: number;
  readonly neutralChroma: number;
  readonly neutralHue?: number;
}

const LIGHT: Tones = {
  dark: false,
  neutralChroma: 4,
  calloutFg: 30,
  calloutBg: 94,
  roles: {
    primary: ["p", 40],
    "on-primary": ["p", 100],
    "primary-container": ["p", 90],
    "on-primary-container": ["p", 10],
    "secondary-container": ["s", 90],
    "on-secondary-container": ["s", 10],
    surface: ["n", 98],
    "surface-container-lowest": ["n", 100],
    "surface-container-low": ["n", 96],
    "surface-container": ["n", 94],
    "surface-container-high": ["n", 92],
    "surface-container-highest": ["n", 90],
    "on-surface": ["n", 10],
    "on-surface-variant": ["nv", 30],
    outline: ["nv", 50],
    "outline-variant": ["nv", 80],
    "inverse-surface": ["n", 20],
    "inverse-on-surface": ["n", 95],
    error: ["e", 40],
    "on-error": ["e", 100],
    scrim: ["n", 0],
  },
};

const DARK: Tones = {
  dark: true,
  neutralChroma: 4,
  calloutFg: 85,
  calloutBg: 20,
  roles: {
    primary: ["p", 80],
    "on-primary": ["p", 20],
    "primary-container": ["p", 30],
    "on-primary-container": ["p", 90],
    "secondary-container": ["s", 30],
    "on-secondary-container": ["s", 90],
    surface: ["n", 6],
    "surface-container-lowest": ["n", 4],
    "surface-container-low": ["n", 10],
    "surface-container": ["n", 12],
    "surface-container-high": ["n", 17],
    "surface-container-highest": ["n", 22],
    "on-surface": ["n", 90],
    "on-surface-variant": ["nv", 80],
    outline: ["nv", 60],
    "outline-variant": ["nv", 30],
    "inverse-surface": ["n", 90],
    "inverse-on-surface": ["n", 20],
    error: ["e", 80],
    "on-error": ["e", 20],
    scrim: ["n", 0],
  },
};

const DIM: Tones = {
  ...DARK,
  neutralChroma: 6,
  calloutBg: 26,
  roles: {
    ...DARK.roles,
    surface: ["n", 16],
    "surface-container-lowest": ["n", 12],
    "surface-container-low": ["n", 19],
    "surface-container": ["n", 21],
    "surface-container-high": ["n", 25],
    "surface-container-highest": ["n", 29],
    "outline-variant": ["nv", 40],
    "on-surface": ["n", 92],
    "on-surface-variant": ["nv", 82],
    outline: ["nv", 66],
  },
};

const HIGH_CONTRAST: Tones = {
  dark: false,
  neutralChroma: 0,
  calloutFg: 15,
  calloutBg: 96,
  roles: {
    ...LIGHT.roles,
    primary: ["p", 25],
    "primary-container": ["p", 30],
    "on-primary-container": ["p", 100],
    "secondary-container": ["s", 30],
    "on-secondary-container": ["s", 100],
    surface: ["n", 100],
    "surface-container-lowest": ["n", 100],
    "surface-container-low": ["n", 98],
    "surface-container": ["n", 96],
    "surface-container-high": ["n", 94],
    "surface-container-highest": ["n", 92],
    "on-surface": ["n", 0],
    "on-surface-variant": ["nv", 10],
    outline: ["nv", 20],
    "outline-variant": ["nv", 35],
    error: ["e", 25],
  },
};

const SEPIA: Tones = {
  ...LIGHT,
  neutralChroma: 12,
  neutralHue: 75,
  calloutBg: 92,
  roles: {
    ...LIGHT.roles,
    surface: ["n", 95],
    "surface-container-lowest": ["n", 97],
    "surface-container-low": ["n", 93],
    "surface-container": ["n", 91],
    "surface-container-high": ["n", 88],
    "surface-container-highest": ["n", 85],
    "on-surface-variant": ["nv", 28],
    outline: ["nv", 45],
  },
};

const TONES: Readonly<Record<ResolvedTheme, Tones>> = {
  light: LIGHT,
  dark: DARK,
  dim: DIM,
  "high-contrast": HIGH_CONTRAST,
  sepia: SEPIA,
};

/** The brief's light theme values (§13), used exactly when the seed is the default. */
const BRIEF_LIGHT: Partial<Record<Role, string>> = {
  primary: "#4f5b92",
  "primary-container": "#dde1ff",
  surface: "#f7f8fc",
  "surface-container": "#f0f2f9",
  "surface-container-high": "#e6e8ee",
  outline: "#767680",
  "outline-variant": "#c5c6d0",
  "on-surface": "#1d1b20",
  "on-surface-variant": "#46464f",
};

/** Builds every token of a theme from a seed colour. */
export function buildScheme(theme: ResolvedTheme, seedHex: string = DEFAULT_SEED): Scheme {
  const seed = toLch(parseHex(seedHex) ?? parseHex(DEFAULT_SEED) ?? { r: 0, g: 0, b: 0 });
  const spec = TONES[theme];
  const neutral = { ...seed, h: spec.neutralHue ?? seed.h };
  const palettes = {
    p: (t: number) => tone(seed, Math.max(seed.c, 36), t),
    s: (t: number) => tone(seed, 16, t),
    n: (t: number) => tone(neutral, spec.neutralChroma, t),
    nv: (t: number) => tone(neutral, spec.neutralChroma * 2, t),
    e: (t: number) => tone({ l: 50, c: 60, h: 25 }, 60, t),
  };
  const out: Record<string, string> = {};
  for (const role of ROLES) {
    const [palette, value] = spec.roles[role];
    out[role] = palettes[palette](value);
  }
  if (theme === "light" && seedHex.toLowerCase() === DEFAULT_SEED) {
    Object.assign(out, BRIEF_LIGHT);
  }
  for (const [kind, hue] of Object.entries(CALLOUTS)) {
    const base = hue < 0 ? neutral : { l: 50, c: 40, h: hue };
    const chroma = hue < 0 ? spec.neutralChroma * 2 : 40;
    out[`callout-${kind}`] = tone(base, chroma, spec.calloutFg);
    out[`callout-${kind}-bg`] = tone(base, Math.min(chroma, 18), spec.calloutBg);
  }
  return out;
}

/** Whether the theme is dark, for `color-scheme`. */
export function isDark(theme: ResolvedTheme): boolean {
  return TONES[theme].dark;
}

/** A pair of tokens that must meet a WCAG 2.2 contrast ratio. */
export interface ContrastRule {
  readonly fg: string;
  readonly bg: string;
  readonly min: number;
}

/** The pairs the app relies on: 4.5:1 for text, 3:1 for borders and icons. */
export const CONTRAST_RULES: readonly ContrastRule[] = [
  { fg: "on-surface", bg: "surface", min: 4.5 },
  { fg: "on-surface", bg: "surface-container-highest", min: 4.5 },
  { fg: "on-surface-variant", bg: "surface", min: 4.5 },
  { fg: "on-surface-variant", bg: "surface-container", min: 4.5 },
  { fg: "on-surface-variant", bg: "surface-container-high", min: 4.5 },
  { fg: "primary", bg: "surface", min: 4.5 },
  { fg: "primary", bg: "surface-container-low", min: 4.5 },
  { fg: "on-primary", bg: "primary", min: 4.5 },
  { fg: "on-primary-container", bg: "primary-container", min: 4.5 },
  { fg: "on-secondary-container", bg: "secondary-container", min: 4.5 },
  { fg: "outline", bg: "surface", min: 3 },
  { fg: "error", bg: "surface", min: 4.5 },
  { fg: "inverse-on-surface", bg: "inverse-surface", min: 4.5 },
  ...Object.keys(CALLOUTS).map((kind) => ({
    fg: `callout-${kind}`,
    bg: `callout-${kind}-bg`,
    min: 4.5,
  })),
];

/** A contrast rule a scheme breaks, with the ratio it reached. */
export interface ContrastFailure extends ContrastRule {
  readonly ratio: number;
}

/** Validates a scheme against {@link CONTRAST_RULES} (R-TH-2). */
export function checkContrast(scheme: Scheme): ContrastFailure[] {
  const failures: ContrastFailure[] = [];
  for (const rule of CONTRAST_RULES) {
    const fg = parseHex(scheme[rule.fg] ?? "");
    const bg = parseHex(scheme[rule.bg] ?? "");
    const ratio = fg && bg ? contrast(fg, bg) : 0;
    if (ratio < rule.min) {
      failures.push({ ...rule, ratio });
    }
  }
  return failures;
}
