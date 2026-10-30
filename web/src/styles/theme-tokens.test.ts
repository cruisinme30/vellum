// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

const SOURCE_ROOT = path.resolve(import.meta.dirname, "..");

/** Every stylesheet but the token file, which holds the first-paint values. */
function stylesheets(): [file: string, text: string][] {
  return readdirSync(SOURCE_ROOT, { recursive: true, encoding: "utf8" })
    .filter((file) => file.endsWith(".css") && !file.endsWith("tokens.css"))
    .map((file) => [file, readFileSync(path.join(SOURCE_ROOT, file), "utf8")]);
}

describe("stylesheets", () => {
  // Every screen re-themes (R-TH-1, R-TH-3): colours come from --md-* tokens.
  // A colour that must not change with the theme says why with "fixed:".
  it("name colours only through theme tokens", () => {
    const literal = /#[\da-f]{3,8}\b|\brgba?\(\s*\d|\bhsla?\(\s*\d/i;
    const found = stylesheets().flatMap(([file, text]) =>
      text
        .split("\n")
        .map((line, index) => ({ line, at: `${file}:${String(index + 1)}` }))
        .filter(({ line }) => literal.test(line) && !line.includes("fixed:"))
        .filter(({ line }) => !/rgb\(0 0 0 \/ \d+%\)/.test(line))
        .map(({ at, line }) => `${at}: ${line.trim()}`),
    );
    expect(found).toEqual([]);
  });
});
