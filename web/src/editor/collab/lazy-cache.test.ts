// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { lazyCache } from "./lazy-cache";
import type { UpdateCache } from "./provider";

const UPDATE = new Uint8Array([1, 2, 3]);

function fakeCache() {
  const load = vi.fn<UpdateCache["load"]>(() => Promise.resolve([UPDATE]));
  const append = vi.fn<UpdateCache["append"]>(() => Promise.resolve());
  const cache: UpdateCache = { load, append };
  return { cache, load, append };
}

describe("lazyCache", () => {
  it("does not open until first used, then opens once", async () => {
    const target = fakeCache();
    const open = vi.fn(() => Promise.resolve(target.cache));
    const cache = lazyCache(open);
    expect(open).not.toHaveBeenCalled();
    expect(await cache.load("doc")).toEqual([UPDATE]);
    await cache.append("doc", UPDATE);
    expect(open).toHaveBeenCalledTimes(1);
    expect(target.load).toHaveBeenCalledWith("doc");
    expect(target.append).toHaveBeenCalledWith("doc", UPDATE);
  });

  it("loads nothing and drops appends when the cache cannot open", async () => {
    const open = vi.fn(() => Promise.reject(new Error("blocked")));
    const cache = lazyCache(open);
    expect(await cache.load("doc")).toEqual([]);
    await expect(cache.append("doc", UPDATE)).resolves.toBeUndefined();
    expect(open).toHaveBeenCalledTimes(1);
  });
});
