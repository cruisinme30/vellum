// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { DESTINATIONS } from "./destinations";
import { hasIcon } from "./icon";

const all = Object.entries(DESTINATIONS);

describe("DESTINATIONS", () => {
  it("keys each destination by its id", () => {
    for (const [key, destination] of all) {
      expect(destination.id).toBe(key);
    }
  });

  it("has unique absolute paths, with home at the root", () => {
    const paths = all.map(([, destination]) => destination.path);
    expect(new Set(paths).size).toBe(paths.length);
    expect(paths.every((path) => path.startsWith("/"))).toBe(true);
    expect(DESTINATIONS.home.path).toBe("/");
  });

  it("only uses bundled icons", () => {
    expect(all.filter(([, destination]) => !hasIcon(destination.icon))).toEqual([]);
  });
});
