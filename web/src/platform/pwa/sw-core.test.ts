// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import {
  activateShell,
  type ClientsLike,
  entryKey,
  installShell,
  openNotice,
  readPush,
  respond,
  route,
  type ShellManifest,
  shellCacheName,
  type WindowClientLike,
} from "./sw-core";

const ORIGIN = "https://northstar.dev";
const manifest: ShellManifest = {
  version: "v2",
  entries: [
    { url: "/index.html", revision: "abc" },
    { url: "/assets/index-1.js", revision: "def" },
    { url: "/icons/icon-192.png" },
  ],
};

/** A CacheStorage over plain maps. */
function fakeCaches(initial: Record<string, Record<string, string>> = {}) {
  const stores = new Map<string, Map<string, string>>(
    Object.entries(initial).map(([name, entries]) => [
      name,
      new Map(Object.entries(entries)),
    ]),
  );
  const open = (name: string) => {
    const store = stores.get(name) ?? new Map<string, string>();
    stores.set(name, store);
    return Promise.resolve({
      addAll: (keys: string[]) => {
        for (const key of keys) store.set(key, `body of ${key}`);
        return Promise.resolve();
      },
      match: (key: string) => {
        const body = store.get(key);
        return Promise.resolve(
          body === undefined ? undefined : new Response(body),
        );
      },
    });
  };
  const caches = {
    open,
    keys: () => Promise.resolve([...stores.keys()]),
    delete: (name: string) => Promise.resolve(stores.delete(name)),
  } as unknown as CacheStorage;
  return { caches, stores };
}

const request = (path: string, mode = "cors", method = "GET") => ({
  url: new URL(path, ORIGIN).href,
  mode,
  method,
});

describe("shell cache", () => {
  it("installs every entry under a versioned cache and revisioned keys", async () => {
    const { caches, stores } = fakeCaches();
    await installShell(caches, manifest);
    expect([...(stores.get("vellum-shell-v2")?.keys() ?? [])]).toEqual([
      "/index.html?__rev=abc",
      "/assets/index-1.js?__rev=def",
      "/icons/icon-192.png",
    ]);
    expect(shellCacheName(manifest)).toBe("vellum-shell-v2");
    expect(entryKey({ url: "/a" })).toBe("/a");
  });

  it("deletes only older shell caches on activation", async () => {
    const { caches, stores } = fakeCaches({
      "vellum-shell-v1": {},
      "vellum-shell-v2": {},
      other: {},
    });
    expect(await activateShell(caches, manifest)).toEqual(["vellum-shell-v1"]);
    expect([...stores.keys()]).toEqual(["vellum-shell-v2", "other"]);
  });
});

describe("routing", () => {
  it.each([
    ["/api/v1/sync/pull", "cors", "GET", "network"],
    ["/mcp", "cors", "POST", "network"],
    ["/auth/callback", "navigate", "GET", "network"],
    ["/docs/123", "navigate", "GET", "shell"],
    ["/assets/index-1.js", "cors", "GET", "precache"],
    ["/assets/unknown.js", "cors", "GET", "network"],
    ["/index.html", "cors", "POST", "network"],
  ])("%s (%s %s) goes to %s", (path, mode, method, kind) => {
    expect(route(request(path, mode, method), ORIGIN, manifest).kind).toBe(
      kind,
    );
  });

  it("leaves other origins alone", () => {
    const foreign = {
      url: "https://cdn.example/x.js",
      mode: "cors",
      method: "GET",
    };
    expect(route(foreign, ORIGIN, manifest)).toEqual({ kind: "network" });
  });
});

describe("respond", () => {
  const fetchMock = vi.fn((url: string) =>
    Promise.resolve(new Response(`network ${url}`)),
  );

  it("serves navigations from the cached shell and assets cache-first", async () => {
    const { caches } = fakeCaches();
    await installShell(caches, manifest);
    const context = { caches, origin: ORIGIN, manifest, fetch: fetchMock };
    const shell = await respond(request("/docs/1", "navigate"), context);
    expect(await shell?.text()).toBe("body of /index.html?__rev=abc");
    const asset = await respond(request("/assets/index-1.js"), context);
    expect(await asset?.text()).toBe("body of /assets/index-1.js?__rev=def");
    expect(await respond(request("/api/v1/x"), context)).toBeUndefined();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("falls back to the network on a cache miss", async () => {
    const { caches } = fakeCaches();
    const context = { caches, origin: ORIGIN, manifest, fetch: fetchMock };
    const shell = await respond(request("/docs/1", "navigate"), context);
    expect(await shell?.text()).toBe(`network ${ORIGIN}/docs/1`);
    const asset = await respond(request("/icons/icon-192.png"), context);
    expect(await asset?.text()).toBe("network /icons/icon-192.png");
  });

  it("lets the browser handle navigations when no shell is precached", async () => {
    const { caches } = fakeCaches();
    const bare = { version: "x", entries: [] };
    const context = {
      caches,
      origin: ORIGIN,
      manifest: bare,
      fetch: fetchMock,
    };
    expect(
      await respond(request("/docs/1", "navigate"), context),
    ).toBeUndefined();
  });
});

describe("push", () => {
  it("reads a full payload", () => {
    const payload = JSON.stringify({
      title: "ENG-142",
      body: "Assigned to you",
      url: "/issues/ENG-142?x=1#c",
      tag: "t",
    });
    expect(readPush(payload, ORIGIN)).toEqual({
      title: "ENG-142",
      body: "Assigned to you",
      url: "/issues/ENG-142?x=1#c",
      tag: "t",
    });
  });

  it.each([undefined, "not json", "42", JSON.stringify({ title: 3 })])(
    "shows a generic notice for %s",
    (payload) => {
      expect(readPush(payload, ORIGIN)).toEqual({
        title: "Vellum",
        body: "You have a new notification.",
        tag: undefined,
        url: "/inbox",
      });
    },
  );

  it("never links to another origin", () => {
    const payload = JSON.stringify({
      title: "x",
      url: "https://evil.example/phish",
    });
    expect(readPush(payload, ORIGIN).url).toBe("/inbox");
  });
});

/** A mock body; the URL argument is what the tests assert on. */
function resolvedWith(url: string): Promise<string> {
  return Promise.resolve(url);
}

function windowClient(url: string) {
  const focus = vi.fn(() => Promise.resolve());
  const navigate = vi.fn(resolvedWith);
  const client: WindowClientLike = { url, focus, navigate };
  return { client, focus, navigate };
}

function clientsOf(windows: WindowClientLike[]) {
  const openWindow = vi.fn(resolvedWith);
  const clients: ClientsLike = {
    matchAll: () => Promise.resolve(windows),
    openWindow,
  };
  return { clients, openWindow };
}

describe("notification clicks", () => {
  it("focuses a window already on the page", async () => {
    const exact = windowClient(`${ORIGIN}/inbox`);
    const { clients, openWindow } = clientsOf([
      windowClient(`${ORIGIN}/`).client,
      exact.client,
    ]);
    await openNotice(clients, ORIGIN, "/inbox");
    expect(exact.focus).toHaveBeenCalled();
    expect(openWindow).not.toHaveBeenCalled();
  });

  it("navigates an open window, or opens one", async () => {
    const other = windowClient(`${ORIGIN}/`);
    await openNotice(clientsOf([other.client]).clients, ORIGIN, "/inbox");
    expect(other.navigate).toHaveBeenCalledWith(`${ORIGIN}/inbox`);
    expect(other.focus).toHaveBeenCalled();
    const none = clientsOf([]);
    await openNotice(none.clients, ORIGIN, "/inbox");
    expect(none.openWindow).toHaveBeenCalledWith(`${ORIGIN}/inbox`);
  });
});
