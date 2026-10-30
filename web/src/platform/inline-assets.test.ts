// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { inlineAsset } from "./inline-assets";

describe("inlineAsset", () => {
  it("never inlines fonts, which the CSP only allows from the app's origin", () => {
    for (const file of [
      "/node_modules/@fontsource/roboto/files/roboto-greek-ext-400-normal.woff2",
      "/fonts/a.woff",
      "/fonts/b.TTF",
      "/fonts/c.otf?v=2",
      "/fonts/d.eot",
    ]) {
      expect(inlineAsset(file)).toBe(false);
    }
  });

  it("leaves other assets to Vite's size rule", () => {
    expect(inlineAsset("/icons/check.svg")).toBeUndefined();
    expect(inlineAsset("/img/logo.png")).toBeUndefined();
  });
});
