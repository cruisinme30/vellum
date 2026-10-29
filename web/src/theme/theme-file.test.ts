// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  customScheme,
  exportTheme,
  importTheme,
  THEME_FORMAT,
  themeFileName,
  TOKEN_NAMES,
  type CustomTheme,
} from "./theme-file";
import { buildScheme } from "./themes";

const INK: CustomTheme = { name: "Northstar ink", base: "light", seed: "#225588", tokens: {} };

const file = (patch: Record<string, unknown>) =>
  JSON.stringify({ format: THEME_FORMAT, name: "Ink", base: "dark", seed: "#225588", ...patch });

describe("theme files", () => {
  it("round-trips every token through export and import", () => {
    const theme = { ...INK, tokens: { primary: "#1a4a7a" } };
    const text = exportTheme(theme);
    const parsed = JSON.parse(text) as { tokens: Record<string, string> };
    expect(Object.keys(parsed.tokens)).toEqual(TOKEN_NAMES);
    const result = importTheme(text);
    expect(result.ok).toBe(true);
    if (result.ok) expect(customScheme(result.theme)).toEqual(customScheme(theme));
  });

  it("fills tokens a file leaves out from its start theme and seed", () => {
    const result = importTheme(file({ tokens: { primary: "#AABBFF" } }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const scheme = customScheme(result.theme);
    expect(scheme["primary"]).toBe("#aabbff");
    expect(scheme["surface"]).toBe(buildScheme("dark", "#225588")["surface"]);
  });

  it.each([
    ["too-large", "x".repeat(70_000)],
    ["not-json", "{"],
    ["format", JSON.stringify({ format: "other/1" })],
    ["format", "[]"],
    ["name", file({ name: "  " })],
    ["name", file({ name: "x".repeat(65) })],
    ["name", file({ name: "bad\u0007bell" })],
    ["base", file({ base: "system" })],
    ["seed", file({ seed: "red" })],
    ["token", file({ tokens: { "made-up": "#000000" } })],
    ["token", file({ tokens: { primary: "blue" } })],
    ["token", file({ tokens: ["#000000"] })],
  ])("refuses a file with a bad %s", (kind, text) => {
    const result = importTheme(text);
    expect(result.ok ? "ok" : result.error.kind).toBe(kind);
  });

  it("refuses a theme whose colours fail contrast and says which", () => {
    const result = importTheme(file({ base: "light", tokens: { "on-surface": "#eeeeee" } }));
    expect(result.ok).toBe(false);
    if (result.ok || result.error.kind !== "contrast") throw new Error("expected contrast");
    expect(result.error.failures.map((f) => f.fg)).toContain("on-surface");
  });

  it("names exported files after the theme", () => {
    expect(themeFileName(INK)).toBe("northstar-ink.vellum-theme.json");
    expect(themeFileName({ ...INK, name: "☆☆" })).toBe("theme.vellum-theme.json");
  });
});
