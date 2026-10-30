// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { ApiError } from "../api/http";
import { node, loaded, searchHit, marked, page } from "../test/workspace-http";

describe("the HTTP workspace store", () => {
  it("searches page text on the server, one hit per tree page", async () => {
    const { http, store } = await loaded({
      "GET /api/v1/search": {
        hits: [
          searchHit("p2", "plan "),
          searchHit("elsewhere", "plan"),
          searchHit("p2", "again"),
          searchHit("p1", "plan"),
        ],
        has_more: false,
        hybrid: false,
      },
    });
    expect(await store.search("  ")).toEqual([]);
    const hits = await store.search(" plan ");
    expect(http.get.mock.lastCall).toEqual([
      "/api/v1/search",
      { q: "plan", kinds: "page", limit: 50 },
    ]);
    expect(hits).toEqual([
      {
        page: page(store, "p2"),
        snippet: "the plan",
        fragments: marked("plan "),
      },
      {
        page: page(store, "p1"),
        snippet: "the plan",
        fragments: marked("plan"),
      },
    ]);
    expect(await store.mentions("priya")).toEqual([]);

    // Filters go to the server and are checked again on the tree's pages.
    const starred = await store.search("plan", {
      starred: true,
      spaceId: "s1",
      modifiedDays: 7,
    });
    const query = http.get.mock.lastCall?.[1];
    expect(query).toMatchObject({ q: "plan", starred: true, spaces: "s1" });
    expect(typeof query?.["modified_from"]).toBe("number");
    expect(starred).toEqual([]);
  });

  it("searches titles when the server runs no search service", async () => {
    const { http, store } = await loaded({
      "GET /api/v1/search": new ApiError("not_found", "no route", {
        status: 404,
      }),
      "GET /api/v1/drive/search": {
        items: [node("p2", "page"), node("elsewhere", "page")],
      },
    });
    const hits = await store.search(" p ");
    expect(http.get.mock.lastCall).toEqual([
      "/api/v1/drive/search",
      { title: "p", limit: 50 },
    ]);
    expect(hits).toEqual([{ page: page(store, "p2"), snippet: "" }]);
    expect(store.getSnapshot().problem).toBeUndefined();
  });

  it("reports a search that fails", async () => {
    const { store } = await loaded({ "GET /api/v1/search": new Error("down") });
    expect(await store.search("x")).toEqual([]);
    expect(store.getSnapshot().problem?.message).toBe("down");
  });
});
