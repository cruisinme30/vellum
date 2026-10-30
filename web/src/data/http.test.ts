// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/http";
import {
  PRIYA,
  SAM,
  T,
  node,
  fakeHttp,
  BASE,
  loaded,
  settled,
  page,
} from "../test/workspace-http";

import { createHttpWorkspaceStore } from "./http";

describe("the HTTP workspace store", () => {
  it("is empty and idle until someone signs in", () => {
    const http = fakeHttp({});
    const store = createHttpWorkspaceStore(http);
    expect(store.getSnapshot()).toMatchObject({
      name: "Vellum",
      pages: [],
      loading: false,
    });
    store.setUser?.(undefined);
    expect(http.get.mock.calls).toHaveLength(0);
    expect(store.seedFor("p1")).toBeUndefined();
  });

  it("loads the spaces and their folder and page trees", async () => {
    const http = fakeHttp(BASE);
    const store = createHttpWorkspaceStore(http, {
      user: PRIYA,
      name: "Northstar",
    });
    const listener = vi.fn();
    store.subscribe(listener);
    expect(store.getSnapshot()).toMatchObject({
      loading: true,
      me: { handle: "priya" },
    });
    await settled(store);
    const snapshot = store.getSnapshot();
    expect(listener).toHaveBeenCalled();
    expect(snapshot.name).toBe("Northstar");
    expect(snapshot.people).toEqual([{ handle: "priya", name: "Priya Raman" }]);
    expect(snapshot.spaces).toEqual([
      { id: "S-MINE", slug: "priya", name: "My Drive", icon: "🏠" },
      { id: "S-TEAM", slug: "eng", name: "Engineering", icon: "👥" },
    ]);
    expect(snapshot.pages.map((p) => p.id)).toEqual(["f1", "p2", "p1", "t1"]);
    expect(http.get.mock.calls).toContainEqual([
      "/api/v1/drive/spaces/S-MINE/tree",
      { limit: 10_000 },
    ]);
    expect(page(store, "f1")).toMatchObject({
      folder: true,
      icon: "📁",
      position: 1,
    });
    expect(page(store, "p1")).toEqual({
      id: "p1",
      spaceId: "S-MINE",
      parentId: undefined,
      position: 2,
      title: "p1",
      icon: "📄",
      status: "draft",
      owner: "priya",
      updatedAt: T,
      starred: false,
    });
    expect(page(store, "p2")).toMatchObject({
      parentId: "f1",
      starred: true,
      owner: "",
    });
  });

  it(
    "loads level by level when the tree is cut short, nesting pages under " +
      "pages",
    async () => {
      const { store } = await loaded({
        "GET /api/v1/drive/spaces/S-MINE/tree": {
          items: [node("f1", "folder")],
          truncated: true,
        },
        "GET /api/v1/drive/spaces/S-MINE/items": {
          items: [node("f1", "folder"), node("p1", "page"), node("b1", "book")],
        },
        "GET /api/v1/drive/spaces/S-MINE/items?parent_id=f1": {
          items: [node("p2", "page", "f1")],
        },
        "GET /api/v1/drive/spaces/S-MINE/items?parent_id=p1": {
          items: [node("sub", "page", "p1")],
        },
        "GET /api/v1/drive/spaces/S-MINE/items?parent_id=p2": { items: [] },
        "GET /api/v1/drive/spaces/S-MINE/items?parent_id=sub": { items: [] },
      });
      expect(store.getSnapshot().pages.map((p) => p.id)).toEqual([
        "f1",
        "p1",
        "p2",
        "sub",
        "t1",
      ]);
      expect(page(store, "sub")).toMatchObject({ parentId: "p1", position: 1 });
    },
  );

  it("never makes a My Drive itself, and reports one missing", async () => {
    const { http, store } = await loaded({
      "GET /api/v1/drive/my-drive": JSON.parse('{ "space": null }') as unknown,
    });
    expect(http.send.mock.calls).toHaveLength(0);
    expect(store.getSnapshot()).toMatchObject({
      loading: false,
      spaces: [],
      problem: { message: "The server returned no My Drive." },
    });
  });

  it("reports a load that fails, with the request ID", async () => {
    const { store } = await loaded({
      "GET /api/v1/drive/shared-drives": new ApiError(
        "permission_denied",
        "not allowed",
        {
          requestId: "R1",
        },
      ),
    });
    expect(store.getSnapshot()).toMatchObject({
      loading: false,
      spaces: [],
      problem: { message: "not allowed", requestId: "R1" },
    });
    store.dismissProblem();
    expect(store.getSnapshot().problem).toBeUndefined();
    const before = store.getSnapshot();
    store.dismissProblem();
    expect(store.getSnapshot()).toBe(before);
  });

  it("reports errors that are not from the server too", async () => {
    const { store } = await loaded({
      "GET /api/v1/drive/starred": new Error("offline"),
    });
    expect(store.getSnapshot().problem).toEqual({ message: "offline" });
    const strange = await loaded({
      "GET /api/v1/drive/starred": () =>
        // A thrown value that is not an Error still gets a message.
        // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
        Promise.reject("odd"),
    });
    expect(strange.store.getSnapshot().problem).toEqual({ message: "odd" });
  });

  it(
    "reloads for a new person, drops a stale answer and empties on " +
      "sign-out",
    async () => {
      const { http, store } = await loaded();
      store.setUser?.(PRIYA);
      expect(store.getSnapshot().loading).toBe(false);
      store.setUser?.(SAM);
      store.setUser?.(undefined);
      expect(store.getSnapshot()).toMatchObject({
        loading: false,
        pages: [],
        me: { handle: "" },
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(store.getSnapshot().pages).toEqual([]);
      const failing = fakeHttp({
        "GET /api/v1/drive/my-drive": new Error("late"),
      });
      const other = createHttpWorkspaceStore(failing, { user: PRIYA });
      other.setUser?.(undefined);
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(other.getSnapshot().problem).toBeUndefined();
      expect(http.get.mock.calls.length).toBeGreaterThan(0);
    },
  );

  it("creates a page and answers with the server's ID", async () => {
    const { http, store } = await loaded({
      "POST /api/v1/drive/spaces/S-MINE/pages": node("new", "page", "f1", {
        title: "",
      }),
    });
    const created = await store.createPage({
      spaceId: "S-MINE",
      parentId: "f1",
    });
    expect(http.send.mock.lastCall).toEqual([
      "POST",
      "/api/v1/drive/spaces/S-MINE/pages",
      {
        parent_id: "f1",
        title: "",
      },
    ]);
    expect(created).toMatchObject({
      id: "new",
      parentId: "f1",
      position: 2,
      owner: "priya",
    });
    expect(page(store, "new")).toEqual(created);
  });

  it("reports a page it could not create", async () => {
    const { store } = await loaded({
      "POST /api/v1/drive/spaces/S-MINE/pages": new ApiError(
        "invalid_input",
        "not a folder",
      ),
    });
    expect(
      await store.createPage({ spaceId: "S-MINE", parentId: "p1", title: "x" }),
    ).toBeUndefined();
    expect(store.getSnapshot().problem?.message).toBe("not a folder");
  });

  it("renames at once and takes the server's title", async () => {
    const { http, store } = await loaded({
      "PATCH /api/v1/drive/items/p1": node("p1", "page", undefined, {
        title: "Plan",
        modified: "2026-10-29T11:00:00Z",
      }),
    });
    store.renamePage("p1", " Plan ");
    expect(page(store, "p1")?.title).toBe(" Plan ");
    await settled(store);
    expect(http.send.mock.lastCall).toEqual([
      "PATCH",
      "/api/v1/drive/items/p1",
      {
        title: " Plan ",
      },
    ]);
    expect(page(store, "p1")).toMatchObject({
      title: "Plan",
      updatedAt: "2026-10-29T11:00:00Z",
    });
  });

  it("puts a refused rename back and says why", async () => {
    const { store } = await loaded({
      "PATCH /api/v1/drive/items/p1": new ApiError("conflict", "name taken", {
        requestId: "R2",
      }),
    });
    store.renamePage("p1", "Clash");
    await settled(store);
    expect(page(store, "p1")?.title).toBe("p1");
    expect(store.getSnapshot().problem).toEqual({
      message: "name taken",
      requestId: "R2",
    });
  });

  it("stars and unstars, undoing a refusal", async () => {
    const calls: unknown[] = [];
    const { store } = await loaded({
      "PUT /api/v1/drive/items/p1/star": (body: unknown) => {
        calls.push(body);
        return calls.length === 1
          ? { ok: true }
          : new ApiError("internal", "boom");
      },
    });
    store.toggleStar("p1");
    await settled(store);
    expect(page(store, "p1")?.starred).toBe(true);
    store.toggleStar("p1");
    expect(page(store, "p1")?.starred).toBe(false);
    await settled(store);
    expect(page(store, "p1")?.starred).toBe(true);
    expect(calls).toEqual([{ starred: true }, { starred: false }]);
    store.toggleStar("missing");
    expect(calls).toHaveLength(2);
  });

  it(
    "moves and reorders on the server, sending the place among the new " +
      "siblings",
    async () => {
      const { http, store } = await loaded({
        "POST /api/v1/drive/items/p1/move": node("p1", "page", "f1"),
      });
      store.movePage("p1", {
        spaceId: "S-MINE",
        parentId: undefined,
        index: 0,
      });
      await settled(store);
      expect(http.send.mock.lastCall).toEqual([
        "POST",
        "/api/v1/drive/items/p1/move",
        { space_id: "S-MINE", parent_id: undefined, index: 0 },
      ]);
      expect(page(store, "p1")?.position).toBe(1);
      store.movePage("p1", { spaceId: "S-MINE", parentId: "f1", index: 1 });
      await settled(store);
      expect(http.send.mock.lastCall).toEqual([
        "POST",
        "/api/v1/drive/items/p1/move",
        { space_id: "S-MINE", parent_id: "f1", index: 1 },
      ]);
      expect(page(store, "p1")?.parentId).toBe("f1");
      store.movePage("missing", {
        spaceId: "S-MINE",
        parentId: undefined,
        index: 0,
      });
      expect(http.send.mock.calls).toHaveLength(2);
    },
  );

  it("undoes a move the server refuses", async () => {
    const { store } = await loaded({
      "POST /api/v1/drive/items/f1/move": new ApiError(
        "invalid_input",
        "not a folder",
      ),
    });
    store.movePage("f1", { spaceId: "S-MINE", parentId: "p1", index: 0 });
    expect(page(store, "f1")?.parentId).toBe("p1");
    await settled(store);
    expect(page(store, "f1")).toMatchObject({
      parentId: undefined,
      position: 1,
    });
    expect(page(store, "p2")?.parentId).toBe("f1");
  });

  it(
    "trashes a folder with what is in it, and brings it back on " + "failure",
    async () => {
      const answers = [
        { id: "f1" },
        new ApiError("permission_denied", "read only"),
      ];
      const { store } = await loaded({
        "POST /api/v1/drive/items/f1/trash": () => answers.shift(),
      });
      store.trashPage("f1");
      expect(store.getSnapshot().pages.map((p) => p.id)).toEqual(["p1", "t1"]);
      await settled(store);
      expect(store.getSnapshot().pages.map((p) => p.id)).toEqual(["p1", "t1"]);
      const second = await loaded({
        "POST /api/v1/drive/items/f1/trash": new ApiError(
          "permission_denied",
          "read only",
        ),
      });
      second.store.trashPage("f1");
      await settled(second.store);
      expect(
        second.store
          .getSnapshot()
          .pages.map((p) => p.id)
          .toSorted((a, b) => a.localeCompare(b)),
      ).toEqual(["f1", "p1", "p2", "t1"]);
      expect(second.store.getSnapshot().problem?.message).toBe("read only");
    },
  );

  it("stops telling a listener once it unsubscribes", async () => {
    const { store } = await loaded({
      "PUT /api/v1/drive/items/p1/star": { ok: true },
    });
    const listener = vi.fn();
    const stop = store.subscribe(listener);
    store.toggleStar("p1");
    expect(listener).toHaveBeenCalledOnce();
    stop();
    store.toggleStar("p1");
    expect(listener).toHaveBeenCalledOnce();
  });
});
