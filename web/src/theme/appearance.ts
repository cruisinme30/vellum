// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { customScheme, validateTheme, type CustomTheme } from "./theme-file";
import {
  buildScheme,
  DEFAULT_SEED,
  isDark,
  THEMES,
  type ResolvedTheme,
  type ThemeName,
} from "./themes";

/** Interface density (R-TH-5). */
export const DENSITIES = ["comfortable", "compact", "spacious"] as const;
type Density = (typeof DENSITIES)[number];

/** Motion: follow the system setting, or always reduce, or always allow (R-TH-5). */
export const MOTIONS = ["system", "reduce", "full"] as const;
type Motion = (typeof MOTIONS)[number];

/** Reading fonts: Roboto, Atkinson Hyperlegible, or OpenDyslexic (R-TH-5). */
export const FONTS = ["default", "hyperlegible", "dyslexic"] as const;
export type ReadingFont = (typeof FONTS)[number];

/** Text sizes, as a percentage of the browser's size (R-TH-5). */
export const TEXT_SIZES = [87.5, 100, 112.5, 125, 150, 175] as const;

/** A theme choice: a built-in theme, or the user's custom theme. */
export type ThemeChoice = ThemeName | "custom";

/** A user's appearance settings. */
export interface Appearance {
  readonly theme: ThemeChoice;
  /** The user's accent colour, `#rrggbb`; absent follows the organisation's. */
  readonly seed: string | undefined;
  readonly density: Density;
  readonly motion: Motion;
  /** Percentage of the browser's text size, one of {@link TEXT_SIZES}. */
  readonly textSize: number;
  readonly font: ReadingFont;
  /** The custom theme, once the user has made or imported one. */
  readonly custom: CustomTheme | undefined;
}

/** What a new user sees. */
export const DEFAULT_APPEARANCE: Appearance = {
  theme: "system",
  seed: undefined,
  density: "comfortable",
  motion: "system",
  textSize: 100,
  font: "default",
  custom: undefined,
};

const STORAGE_KEY = "vellum.appearance";

const pick = <T extends string>(options: readonly T[], value: unknown, fallback: T): T =>
  options.find((option) => option === value) ?? fallback;

function readCustom(value: unknown): CustomTheme | undefined {
  if (value === undefined) return undefined;
  // Stored themes pass the same checks as imported files.
  const result = validateTheme(value);
  return result.ok ? result.theme : undefined;
}

/**
 * A saved accent colour. Vellum's own seed counts as "not chosen": settings
 * saved before organisation colours existed always held it, and those
 * users should now follow their organisation's colour.
 */
function readSeed(value: unknown): string | undefined {
  if (typeof value !== "string" || !/^#[\da-f]{6}$/i.test(value)) return undefined;
  const seed = value.toLowerCase();
  return seed === DEFAULT_SEED ? undefined : seed;
}

/** Reads one stored settings object, keeping what is valid. */
function readAppearance(value: Record<string, unknown>): Appearance {
  const custom = readCustom(value["custom"]);
  const choices: readonly ThemeChoice[] = custom === undefined ? THEMES : [...THEMES, "custom"];
  const size = TEXT_SIZES.find((each) => each === value["textSize"]);
  return {
    theme: pick(choices, value["theme"], DEFAULT_APPEARANCE.theme),
    seed: readSeed(value["seed"]),
    density: pick(DENSITIES, value["density"], DEFAULT_APPEARANCE.density),
    motion: pick(MOTIONS, value["motion"], DEFAULT_APPEARANCE.motion),
    textSize: size ?? DEFAULT_APPEARANCE.textSize,
    font: pick(FONTS, value["font"], DEFAULT_APPEARANCE.font),
    custom,
  };
}

/**
 * Reads saved settings; anything unreadable falls back to the defaults.
 * With nothing saved, a theme already set on the page (`<html
 * data-theme>`, as the UI test suite does before load) is the start theme.
 */
export function loadAppearance(
  storage: Pick<Storage, "getItem"> | undefined,
  pageTheme?: string,
): Appearance {
  const preset = THEMES.find((t) => t === pageTheme);
  const fallback = preset ? { ...DEFAULT_APPEARANCE, theme: preset } : DEFAULT_APPEARANCE;
  try {
    const raw: unknown = JSON.parse(storage?.getItem(STORAGE_KEY) ?? "null");
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
      return fallback;
    }
    return readAppearance(raw as Record<string, unknown>);
  } catch {
    return fallback;
  }
}

/** Saves settings; a browser that refuses storage keeps them for the session only. */
export function saveAppearance(storage: Pick<Storage, "setItem"> | undefined, value: Appearance) {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(value));
  } catch {
    // Private windows and blocked storage: the setting lasts until reload.
  }
}

/** Replaces `system` with the OS preference, and `custom` with its start theme. */
export function resolveTheme(
  theme: ThemeChoice,
  prefersDark: boolean,
  custom?: CustomTheme,
): ResolvedTheme {
  if (theme === "custom") {
    return custom?.base ?? (prefersDark ? "dark" : "light");
  }
  if (theme === "system") {
    return prefersDark ? "dark" : "light";
  }
  return theme;
}

/** The accent colour in use: the user's, else the organisation's, else Vellum's. */
export function effectiveSeed(value: Appearance, organisationSeed?: string): string {
  return value.seed ?? organisationSeed ?? DEFAULT_SEED;
}

/** Loads a reading font's faces the first time it is chosen. */
const FONT_LOADERS: Readonly<Record<ReadingFont, () => Promise<unknown>>> = {
  default: () => Promise.resolve(),
  hyperlegible: () =>
    Promise.all([
      import("@fontsource/atkinson-hyperlegible/400.css"),
      import("@fontsource/atkinson-hyperlegible/700.css"),
    ]),
  dyslexic: () =>
    Promise.all([
      import("@fontsource/opendyslexic/400.css"),
      import("@fontsource/opendyslexic/700.css"),
    ]),
};

/** Fetches a reading font's stylesheet; a failure leaves the fallback fonts. */
export function loadReadingFont(font: ReadingFont): void {
  FONT_LOADERS[font]().catch(() => {
    // Offline before the font was cached: the font stack falls back.
  });
}

/** Writes the theme tokens and reading settings onto an element, normally `<html>`. */
export function applyAppearance(
  root: HTMLElement,
  value: Appearance,
  prefersDark: boolean,
  organisationSeed?: string,
) {
  const custom = value.theme === "custom" ? value.custom : undefined;
  const theme = resolveTheme(value.theme, prefersDark, custom);
  const scheme =
    custom === undefined
      ? buildScheme(theme, effectiveSeed(value, organisationSeed))
      : customScheme(custom);
  for (const [token, color] of Object.entries(scheme)) {
    root.style.setProperty(`--md-${token}`, color);
  }
  root.dataset["theme"] = theme;
  root.dataset["density"] = value.density;
  root.dataset["motion"] = value.motion;
  root.dataset["font"] = value.font;
  if (custom === undefined) {
    delete root.dataset["customTheme"];
  } else {
    root.dataset["customTheme"] = custom.name;
  }
  if (value.textSize === 100) {
    root.style.removeProperty("font-size");
  } else {
    root.style.fontSize = `${String(value.textSize)}%`;
  }
  root.style.colorScheme = isDark(theme) ? "dark" : "light";
}
