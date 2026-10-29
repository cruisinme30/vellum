// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { formatToday } from "./today";

describe("formatToday", () => {
  it("formats a local date with zero padding", () => {
    expect(formatToday(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
    expect(formatToday(new Date(2026, 11, 31))).toBe("2026-12-31");
  });

  it("defaults to now", () => {
    expect(formatToday()).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
});
