// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  buildScheme,
  DEFAULT_SEED,
  isDark,
  THEMES,
  type ResolvedTheme,
  type ThemeName,
} from "./themes";

/** Interface density (R-TH-5). */
export type Density = "comfortable" | "compact";

/** A user's appearance settings. */
export interface Appearance {
  readonly theme: ThemeName;
  /** The organisation or user seed colour, `#rrggbb`. */
  readonly seed: string;
  readonly density: Density;
}

/** What a new user sees. */
export const DEFAULT_APPEARANCE: Appearance = {
  theme: "system",
  seed: DEFAULT_SEED,
  density: "comfortable",
};

const STORAGE_KEY = "vellum.appearance";

/** Reads saved settings; anything unreadable falls back to the defaults. */
export function loadAppearance(storage: Pick<Storage, "getItem"> | undefined): Appearance {
  try {
    const raw: unknown = JSON.parse(storage?.getItem(STORAGE_KEY) ?? "null");
    if (typeof raw !== "object" || raw === null) {
      return DEFAULT_APPEARANCE;
    }
    const value = raw as Record<string, unknown>;
    const theme = THEMES.find((t) => t === value["theme"]) ?? DEFAULT_APPEARANCE.theme;
    const seed =
      typeof value["seed"] === "string" && /^#[\da-f]{6}$/i.test(value["seed"])
        ? value["seed"].toLowerCase()
        : DEFAULT_SEED;
    const density = value["density"] === "compact" ? "compact" : "comfortable";
    return { theme, seed, density };
  } catch {
    return DEFAULT_APPEARANCE;
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

/** Replaces `system` with the OS preference. */
export function resolveTheme(theme: ThemeName, prefersDark: boolean): ResolvedTheme {
  if (theme === "system") {
    return prefersDark ? "dark" : "light";
  }
  return theme;
}

/** Writes the theme tokens onto an element, normally `<html>`. */
export function applyAppearance(root: HTMLElement, value: Appearance, prefersDark: boolean) {
  const theme = resolveTheme(value.theme, prefersDark);
  const scheme = buildScheme(theme, value.seed);
  for (const [token, color] of Object.entries(scheme)) {
    root.style.setProperty(`--md-${token}`, color);
  }
  root.dataset["theme"] = theme;
  root.dataset["density"] = value.density;
  root.style.colorScheme = isDark(theme) ? "dark" : "light";
}
