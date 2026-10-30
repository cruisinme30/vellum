// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  formatBlockId,
  hashedBlockId,
  isBlockId,
  newBlockId,
} from "./block-id";

describe("formatBlockId", () => {
  it("writes the low 40 bits as eight Crockford characters", () => {
    expect(formatBlockId(0)).toBe("blk-00000000");
    expect(formatBlockId(31)).toBe("blk-0000000z");
    expect(formatBlockId(2 ** 40 - 1)).toBe("blk-zzzzzzzz");
  });

  it("wraps above 40 bits and ignores sign and fractions", () => {
    expect(formatBlockId(2 ** 40 + 1)).toBe("blk-00000001");
    expect(formatBlockId(-32.7)).toBe("blk-00000010");
  });

  it("formats a value that is not a finite number as zero", () => {
    expect(formatBlockId(Number.NaN)).toBe("blk-00000000");
    expect(formatBlockId(Number.POSITIVE_INFINITY)).toBe("blk-00000000");
  });
});

describe("isBlockId", () => {
  it("accepts canonical ids only", () => {
    expect(isBlockId("blk-0123abcd")).toBe(true);
    for (const value of [
      "blk-0123ABCD",
      "blk-0123abc",
      "blk-0123abci",
      "0123abcd",
      42,
      undefined,
    ]) {
      expect(isBlockId(value), String(value)).toBe(false);
    }
  });
});

describe("newBlockId", () => {
  it("makes canonical, varied ids", () => {
    const ids = new Set(Array.from({ length: 50 }, () => newBlockId()));
    for (const id of ids) {
      expect(isBlockId(id), id).toBe(true);
    }
    expect(ids.size).toBeGreaterThan(45);
  });
});

describe("hashedBlockId", () => {
  it("is stable, canonical and seed-dependent", () => {
    expect(hashedBlockId("page:1")).toBe(hashedBlockId("page:1"));
    expect(hashedBlockId("page:1")).not.toBe(hashedBlockId("page:2"));
    expect(isBlockId(hashedBlockId(""))).toBe(true);
    expect(isBlockId(hashedBlockId("ünïcødé 🚀"))).toBe(true);
  });
});
