// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { deepLinkPath, findDeepLink } from "./deep-link.js";

describe("deep links", () => {
  it("maps vellum:// links to app paths", () => {
    expect(deepLinkPath("vellum://p/abc?x=1")).toBe("/p/abc?x=1");
    expect(deepLinkPath("vellum://inbox/")).toBe("/inbox");
    expect(deepLinkPath("vellum://")).toBe("/");
  });

  it("rejects other schemes and unsafe paths", () => {
    expect(deepLinkPath("https://p/abc")).toBeUndefined();
    expect(deepLinkPath("vellum://p/%2e%2e/../../x")).toBe("/p/x");
    expect(deepLinkPath("vellum://p/a b")).toBe("/p/a%20b");
    expect(deepLinkPath("vellum://p/a:b")).toBeUndefined();
    expect(deepLinkPath("vellum://p/a'b")).toBeUndefined();
    expect(deepLinkPath("not a url")).toBeUndefined();
  });

  it("finds the link in a command line", () => {
    expect(findDeepLink(["/app/Vellum", "--flag", "vellum://search"])).toBe(
      "/search",
    );
    expect(findDeepLink(["/app/Vellum"])).toBeUndefined();
  });
});
