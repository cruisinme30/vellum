// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { afterEach, describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";

import { createHttpCollectionsStore, REFRESH_MS, type Freshness } from "./http";

function fakeHttp() {
  const http = {
    get: vi.fn((path: string, query?: unknown) =>
      Promise.resolve({ path, query }),
    ),
    send: vi.fn((method: string, path: string, body?: unknown) =>
      Promise.resolve({ method, path, body }),
    ),
    forgetSession: vi.fn(),
  };
  return http as typeof http & Http;
}

describe("the HTTP collections store", () => {
  it("maps reads to the REST operations", async () => {
    const http = fakeHttp();
    const store = createHttpCollectionsStore(http, {});
    await store.list();
    await store.info("C 1");
    await store.query({
      collection_id: "C1",
      view: "Board",
      filter: ["status is Done"],
    });
    await store.getRow({ collection_id: "C1", row_id: "R/1" });
    expect(http.get.mock.calls).toEqual([
      ["/api/v1/collections"],
      ["/api/v1/collections/C%201"],
      [
        "/api/v1/collections/C1/query",
        { view: "Board", filter: ["status is Done"] },
      ],
      ["/api/v1/collections/C1/rows/R%2F1"],
    ]);
  });

  it("maps writes and tells open views to load again", async () => {
    const http = fakeHttp();
    const store = createHttpCollectionsStore(http, {});
    const listener = vi.fn();
    const stop = store.subscribe(listener);
    const create = {
      collection_id: "C1",
      values: { name: { type: "text", value: "A" } },
    } as const;
    await store.createRow(create);
    const update = {
      collection_id: "C1",
      row_id: "R1",
      body: "Notes",
    } as const;
    await store.updateRow(update);
    expect(http.send.mock.calls).toEqual([
      ["POST", "/api/v1/collections/C1/rows", create],
      ["PATCH", "/api/v1/collections/C1/rows/R1", update],
    ]);
    expect(listener).toHaveBeenCalledTimes(2);
    stop();
    await store.updateRow(update);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("does not notify when a write fails", async () => {
    const http = fakeHttp();
    http.send.mockRejectedValueOnce(new Error("conflict"));
    const store = createHttpCollectionsStore(http, {});
    const listener = vi.fn();
    store.subscribe(listener);
    await expect(
      store.updateRow({ collection_id: "C1", row_id: "R1" }),
    ).rejects.toThrow("conflict");
    expect(listener).not.toHaveBeenCalled();
  });
});

describe("the HTTP collections store's other operations", () => {
  it("maps schema, CSV and embed operations to their routes", async () => {
    const http = fakeHttp();
    const store = createHttpCollectionsStore(http, {});
    await store.deleteRow({ collection_id: "C1", row_id: "R 1" });
    await store.saveView({
      collection_id: "C1",
      view: "By owner",
      kind: "board",
      group: "owner",
    });
    await store.deleteView({ collection_id: "C1", view: "Old/one" });
    await store.addProperty({
      collection_id: "C1",
      name: "Stage",
      type: "select",
    });
    await store.updateProperty({
      collection_id: "C1",
      key: "stage",
      name: "Phase",
    });
    await store.deleteProperty({ collection_id: "C1", key: "stage" });
    expect(
      http.send.mock.calls.map(([method, path]) => `${method} ${path}`),
    ).toEqual([
      "DELETE /api/v1/collections/C1/rows/R%201",
      "PUT /api/v1/collections/C1/views/By%20owner",
      "DELETE /api/v1/collections/C1/views/Old%2Fone",
      "POST /api/v1/collections/C1/properties",
      "PATCH /api/v1/collections/C1/properties/stage",
      "DELETE /api/v1/collections/C1/properties/stage",
    ]);
    expect(http.send.mock.calls[1]?.[2]).toEqual({
      collection_id: "C1",
      view: "By owner",
      kind: "board",
      group: "owner",
    });
    await store.exportCsv({ collection_id: "C1", view: "Open" });
    await store.embed("id: C1\nview: Open");
    expect(http.get.mock.calls).toEqual([
      ["/api/v1/collections/C1/csv", { view: "Open" }],
      ["/api/v1/collections/embed", { fence: "id: C1\nview: Open" }],
    ]);
  });

  it("tells views to reload after a CSV import writes, not after a dry run", async () => {
    const http = fakeHttp();
    let written = false;
    http.send.mockImplementation(
      (method: string, path: string, body?: unknown) =>
        Promise.resolve({ method, path, body, written }),
    );
    const store = createHttpCollectionsStore(http, {});
    const listener = vi.fn();
    store.subscribe(listener);
    await store.importCsv({
      collection_id: "C1",
      csv: "Name\nA",
      dry_run: true,
    });
    await store.createFromCsv({
      folder: "drives/x",
      title: "T",
      csv: "Name\n",
      dry_run: true,
    });
    expect(listener).not.toHaveBeenCalled();
    written = true;
    await store.importCsv({ collection_id: "C1", csv: "Name\nA" });
    await store.createFromCsv({
      folder: "drives/x",
      title: "T",
      csv: "Name\n",
    });
    expect(listener).toHaveBeenCalledTimes(2);
    expect(
      http.send.mock.calls
        .map(([method, path]) => `${method} ${path}`)
        .slice(-2),
    ).toEqual(["POST /api/v1/collections/C1/csv", "POST /api/v1/collections"]);
  });
});

function page() {
  const handlers = new Map<string, () => void>();
  let visibilityState: DocumentVisibilityState = "visible";
  let clock = 0;
  const target = {
    addEventListener: vi.fn((type: string, handler: () => void) => {
      handlers.set(type, handler);
    }),
    removeEventListener: vi.fn((type: string) => {
      handlers.delete(type);
    }),
  };
  const freshness: Freshness = {
    document: {
      get visibilityState() {
        return visibilityState;
      },
      ...target,
    },
    window: target,
    now: () => clock,
  };
  return {
    freshness,
    handlers,
    hide: () => {
      visibilityState = "hidden";
    },
    show: () => {
      visibilityState = "visible";
    },
    tick: (ms: number) => {
      clock += ms;
      vi.advanceTimersByTime(ms);
    },
  };
}

describe("keeping views fresh", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reloads when the tab comes back and on a slow timer while visible", () => {
    vi.useFakeTimers();
    const p = page();
    const store = createHttpCollectionsStore(fakeHttp(), p.freshness);
    const listener = vi.fn();
    const stop = store.subscribe(listener);
    expect(
      [...p.handlers.keys()].toSorted((a, b) => a.localeCompare(b)),
    ).toEqual(["focus", "visibilitychange"]);

    p.tick(REFRESH_MS);
    expect(listener).toHaveBeenCalledTimes(1);

    // Hidden tabs do not poll; focus right after a reload is skipped.
    p.hide();
    p.tick(REFRESH_MS);
    expect(listener).toHaveBeenCalledTimes(1);
    p.show();
    p.handlers.get("visibilitychange")?.();
    expect(listener).toHaveBeenCalledTimes(2);
    p.handlers.get("focus")?.();
    expect(listener).toHaveBeenCalledTimes(2);

    // The last listener leaving stops everything.
    stop();
    expect(p.handlers.size).toBe(0);
    p.tick(REFRESH_MS * 3);
    expect(listener).toHaveBeenCalledTimes(2);
  });
});
