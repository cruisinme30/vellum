// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "fake-indexeddb/auto";
import { describe, expect, it } from "vitest";

import { DocumentCache } from "./document-cache";
import { openLocalDatabase } from "./persistence";
import { freshDatabaseName } from "./test-support";

const bytes = (...values: number[]) => new Uint8Array(values);
// IndexedDB clones into another realm's Uint8Array; compare contents.
const plain = (updates: readonly Uint8Array[]) =>
  updates.map((update) => [...update]);

async function setup(budgetBytes = 10) {
  let clock = 0;
  const database = await openLocalDatabase(freshDatabaseName());
  const cache = new DocumentCache(database, {
    budgetBytes,
    now: () => {
      clock += 1;
      return clock;
    },
  });
  return { database, cache };
}

describe("DocumentCache", () => {
  it("returns nothing for an uncached document", async () => {
    const { cache } = await setup();
    expect(await cache.load("doc-a")).toEqual([]);
  });

  it("returns appended updates in order and compacts them", async () => {
    const { cache } = await setup();
    await cache.append("doc-a", bytes(1));
    await cache.append("doc-a", bytes(2, 3));
    await cache.append("doc-b", bytes(9));
    expect(plain(await cache.load("doc-a"))).toEqual(
      plain([bytes(1), bytes(2, 3)]),
    );
    await cache.compact("doc-a", bytes(7, 7));
    expect(plain(await cache.load("doc-a"))).toEqual(plain([bytes(7, 7)]));
    expect(plain(await cache.load("doc-b"))).toEqual(plain([bytes(9)]));
  });

  it("evicts the least recently used documents beyond the budget", async () => {
    const { cache } = await setup(6);
    await cache.append("old", bytes(1, 1, 1));
    await cache.append("new", bytes(2, 2, 2));
    await cache.load("old"); // touch: "new" is now the least recent
    await cache.append("newest", bytes(3, 3, 3));
    expect(await cache.load("new")).toEqual([]);
    expect(plain(await cache.load("old"))).toEqual(plain([bytes(1, 1, 1)]));
    expect(plain(await cache.load("newest"))).toEqual(plain([bytes(3, 3, 3)]));
  });

  it("never evicts documents of a pinned space", async () => {
    const { cache } = await setup(2);
    await cache.pin("space-1");
    await cache.track("pinned-doc", "space-1");
    await cache.append("pinned-doc", bytes(1, 1, 1, 1));
    await cache.append("loose", bytes(2, 2));
    await cache.append("loose-2", bytes(3, 3));
    expect(await cache.pinned()).toEqual(["space-1"]);
    expect(await cache.load("pinned-doc")).toHaveLength(1);
    expect(await cache.load("loose")).toEqual([]);
    await cache.unpin("space-1");
    expect(await cache.load("pinned-doc")).toEqual([]);
  });

  it(
    "keeps size and access time when a known document is tracked " + "again",
    async () => {
      const { database, cache } = await setup(100);
      await cache.append("doc", bytes(1, 2));
      await cache.track("doc", "space-2");
      expect(await database.get("documents", "doc")).toMatchObject({
        spaceId: "space-2",
        bytes: 2,
      });
      await cache.compact("new-doc", bytes(1));
      expect(await database.get("documents", "new-doc")).toMatchObject({
        spaceId: "",
        bytes: 1,
      });
    },
  );

  it("removes a document", async () => {
    const { cache } = await setup();
    await cache.append("doc", bytes(1));
    await cache.remove("doc");
    expect(await cache.load("doc")).toEqual([]);
  });

  it("defaults to a 50 MiB budget and the system clock", async () => {
    const database = await openLocalDatabase(freshDatabaseName());
    const cache = new DocumentCache(database);
    await cache.append("doc", new Uint8Array(1024));
    expect(await cache.evict()).toEqual([]);
  });
});
