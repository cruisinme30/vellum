// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";

import { createHttpCollectionsStore } from "./http";

function fakeHttp() {
  const http = {
    get: vi.fn((path: string, query?: unknown) => Promise.resolve({ path, query })),
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
    const store = createHttpCollectionsStore(http);
    await store.list();
    await store.info("C 1");
    await store.query({ collection_id: "C1", view: "Board", filter: ["status is Done"] });
    await store.getRow({ collection_id: "C1", row_id: "R/1" });
    expect(http.get.mock.calls).toEqual([
      ["/api/v1/collections"],
      ["/api/v1/collections/C%201"],
      ["/api/v1/collections/C1/query", { view: "Board", filter: ["status is Done"] }],
      ["/api/v1/collections/C1/rows/R%2F1"],
    ]);
  });

  it("maps writes and tells open views to load again", async () => {
    const http = fakeHttp();
    const store = createHttpCollectionsStore(http);
    const listener = vi.fn();
    const stop = store.subscribe(listener);
    const create = { collection_id: "C1", values: { name: { type: "text", value: "A" } } } as const;
    await store.createRow(create);
    const update = { collection_id: "C1", row_id: "R1", body: "Notes" } as const;
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
    const store = createHttpCollectionsStore(http);
    const listener = vi.fn();
    store.subscribe(listener);
    await expect(store.updateRow({ collection_id: "C1", row_id: "R1" })).rejects.toThrow(
      "conflict",
    );
    expect(listener).not.toHaveBeenCalled();
  });
});
