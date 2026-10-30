// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Theme files (R-TH-2, ADR 0007 D244): a custom theme as JSON tokens.
 *
 * A file names the built-in theme and seed it starts from, and the tokens
 * it changes. Tokens it leaves out come from that start, so a file made
 * today still loads after a release adds tokens. Every file is checked
 * against the contrast rules before it is used.
 */

import {
  buildScheme,
  checkContrast,
  THEMES,
  type ContrastFailure,
  type ResolvedTheme,
  type Scheme,
} from "./themes";

/** The `format` field of a theme file. */
export const THEME_FORMAT = "vellum.theme/1";

/** The largest theme file accepted, in characters. */
const MAX_FILE = 64 * 1024;

/** A custom theme: a built-in start, a seed, and the tokens it changes. */
export interface CustomTheme {
  readonly name: string;
  readonly base: ResolvedTheme;
  /** `#rrggbb`. */
  readonly seed: string;
  /** Token overrides, `#rrggbb` each. */
  readonly tokens: Readonly<Record<string, string>>;
}

/** Every token a scheme has, in a stable order. */
export const TOKEN_NAMES: readonly string[] = Object.keys(buildScheme("light"));

const KNOWN = new Set(TOKEN_NAMES);
const BASES = THEMES.filter((theme): theme is ResolvedTheme => theme !== "system");
const HEX = /^#[\da-f]{6}$/i;

/** The full scheme of a custom theme. */
export function customScheme(theme: CustomTheme): Scheme {
  return { ...buildScheme(theme.base, theme.seed), ...theme.tokens };
}

/** Why a theme file was refused. */
export type ThemeFileError =
  | { readonly kind: "too-large" }
  | { readonly kind: "not-json" }
  | { readonly kind: "format" }
  | { readonly kind: "name" }
  | { readonly kind: "base" }
  | { readonly kind: "seed" }
  | { readonly kind: "token"; readonly token: string }
  | { readonly kind: "contrast"; readonly failures: readonly ContrastFailure[] };

/** A theme file's result. */
export type ThemeFileResult =
  | { readonly ok: true; readonly theme: CustomTheme }
  | { readonly ok: false; readonly error: ThemeFileError };

const fail = (error: ThemeFileError): ThemeFileResult => ({ ok: false, error });

function readTokens(value: unknown): Record<string, string> | ThemeFileError {
  if (value === undefined) return {};
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { kind: "token", token: "tokens" };
  }
  const tokens: Record<string, string> = {};
  for (const [token, color] of Object.entries(value)) {
    if (!KNOWN.has(token) || typeof color !== "string" || !HEX.test(color)) {
      return { kind: "token", token };
    }
    tokens[token] = color.toLowerCase();
  }
  return tokens;
}

/** A name of 1 to 64 characters with no control characters, trimmed. */
function readName(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const name = value.trim();
  let count = 0;
  for (const character of name) {
    const code = character.codePointAt(0) ?? 0;
    if (code < 32 || code === 127) return undefined;
    count += 1;
  }
  return count === 0 || count > 64 ? undefined : name;
}

/**
 * Checks a parsed theme: shape, names, colours and contrast. Used for
 * imported files and for the custom theme kept in the browser.
 */
export function validateTheme(value: unknown): ThemeFileResult {
  if (typeof value !== "object" || value === null) return fail({ kind: "format" });
  const record = value as Record<string, unknown>;
  if (record["format"] !== THEME_FORMAT) return fail({ kind: "format" });
  const name = readName(record["name"]);
  if (name === undefined) return fail({ kind: "name" });
  const base = BASES.find((theme) => theme === record["base"]);
  if (base === undefined) return fail({ kind: "base" });
  const seed = record["seed"];
  if (typeof seed !== "string" || !HEX.test(seed)) return fail({ kind: "seed" });
  const tokens = readTokens(record["tokens"]);
  if ("kind" in tokens) return fail(tokens as ThemeFileError);
  const theme: CustomTheme = { name, base, seed: seed.toLowerCase(), tokens };
  const failures = checkContrast(customScheme(theme));
  return failures.length > 0 ? fail({ kind: "contrast", failures }) : { ok: true, theme };
}

/** Reads a theme file's text. */
export function importTheme(text: string): ThemeFileResult {
  if (text.length > MAX_FILE) return fail({ kind: "too-large" });
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return fail({ kind: "not-json" });
  }
  return validateTheme(value);
}

/**
 * Writes a theme file with every token, so the file shows the whole theme
 * and another Vellum loads it the same way.
 */
export function exportTheme(theme: CustomTheme): string {
  const scheme = customScheme(theme);
  const tokens = Object.fromEntries(TOKEN_NAMES.map((token) => [token, scheme[token] ?? ""]));
  return `${JSON.stringify(
    { format: THEME_FORMAT, name: theme.name, base: theme.base, seed: theme.seed, tokens },
    undefined,
    2,
  )}\n`;
}

/** A file name for an exported theme. */
export function themeFileName(theme: CustomTheme): string {
  const slug = theme.name
    .toLowerCase()
    .replaceAll(/[^\da-z]+/g, "-")
    .replaceAll(/^-|-$/g, "");
  return `${slug === "" ? "theme" : slug}.vellum-theme.json`;
}
