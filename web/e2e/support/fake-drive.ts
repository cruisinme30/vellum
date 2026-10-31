// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page, Route } from "@playwright/test";

/** An item as the drive's `NodeView` shows it. */
interface Item {
  id: string;
  space_id: string;
  kind: "folder" | "page";
  parent_id?: string;
  title: string;
  modified: string;
  created_by: string;
}

/** A refusal the fake answers one request with. */
interface Refusal {
  readonly status: number;
  readonly code: string;
  readonly message: string;
  readonly request_id: string;
}

/** What the fake drive holds and what it was asked, for assertions. */
export interface FakeDrive {
  readonly items: Item[];
  /** Pages shared with the signed-in person from drives they cannot browse. */
  readonly shared: Item[];
  readonly starred: Set<string>;
  /** "METHOD path" of every change, in order. */
  readonly changes: string[];
  /** Refuses the next change whose "METHOD path" ends with the key. */
  refuseNext(key: string, refusal: Refusal): void;
}

const SPACE = {
  id: "01M6400000SPACE0MYDRIVE001",
  kind: "user_drive",
  slug: "priya",
  name: "My Drive",
};

/**
 * Stands in for the drive of `vellum serve` (`/api/v1/drive/...`): one My
 * Drive holding a folder with a page in it, and the changes the pages
 * screens make.
 */
export async function fakeDrive(page: Page, owner: string): Promise<FakeDrive> {
  const now = "2026-10-29T10:00:00Z";
  const items: Item[] = [
    {
      id: "01M6400000FOLDER0PLANS0001",
      space_id: SPACE.id,
      kind: "folder",
      title: "Plans",
      modified: now,
      created_by: owner,
    },
    {
      id: "01M6400000PAGE00ROADMAP001",
      space_id: SPACE.id,
      kind: "page",
      parent_id: "01M6400000FOLDER0PLANS0001",
      title: "Roadmap",
      modified: now,
      created_by: owner,
    },
  ];
  // A page in someone else's drive, shared with the signed-in person on
  // its own: in "Shared with me", never in their trees (#567).
  const shared: Item[] = [
    {
      id: "01M6400000PAGE00LAUNCHQ101",
      space_id: "01M6400000SPACE0OTHERDRV01",
      kind: "page",
      parent_id: "01M6400000FOLDER0OTHER0001",
      title: "Launch plan Q1",
      modified: now,
      created_by: "01M6409GN2EYWS8Z7NFQQ3OTHR",
    },
  ];
  const starred = new Set<string>();
  const changes: string[] = [];
  const refusals = new Map<string, Refusal>();
  let next = 1;

  const notFound = {
    status: 404,
    json: { code: "not_found", message: "no such item" },
  };

  /** The items under `parent`, depth first, each list in its stored order. */
  const depthFirst = (parent?: string): Item[] =>
    items
      .filter((item) => item.parent_id === parent)
      .flatMap((item) => [item, ...depthFirst(item.id)]);

  /** Moves an item under a new parent, at `index` among its new siblings. */
  const move = (item: Item, body: Record<string, unknown>) => {
    const parent = body["parent_id"];
    const index = body["index"];
    items.splice(items.indexOf(item), 1);
    if (typeof parent === "string") item.parent_id = parent;
    else delete item.parent_id;
    const siblings = items.filter(
      (entry) => entry.parent_id === item.parent_id,
    );
    const before = typeof index === "number" ? siblings[index] : undefined;
    items.splice(
      before === undefined ? items.length : items.indexOf(before),
      0,
      item,
    );
  };

  /** The answer to a read, or `undefined` for an unknown route. */
  const read = (path: string, query: URLSearchParams): unknown => {
    if (path === "/my-drive") return { space: SPACE };
    if (path === "/shared-drives") return { spaces: [] };
    if (path === "/shared-with-me") return { items: shared };
    const one = /^\/items\/([^/]+)$/.exec(path)?.[1];
    if (one !== undefined)
      return [...items, ...shared].find((item) => item.id === one);
    if (path === "/starred")
      return { items: items.filter((item) => starred.has(item.id)) };
    if (path === `/spaces/${SPACE.id}/tree`)
      return { items: depthFirst(), truncated: false };
    if (path === `/spaces/${SPACE.id}/items`) {
      const parent = query.get("parent_id") ?? undefined;
      return { items: items.filter((item) => item.parent_id === parent) };
    }
    if (path === "/search") {
      const title = (query.get("title") ?? "").toLowerCase();
      return {
        items: items.filter((item) => item.title.toLowerCase().includes(title)),
      };
    }
    return undefined;
  };

  const create = (body: Record<string, unknown>): Item => {
    const parent = body["parent_id"];
    const title = body["title"];
    const created: Item = {
      id: `01M6400000PAGE00NEW0000${String(next).padStart(3, "0")}`,
      space_id: SPACE.id,
      kind: "page",
      ...(typeof parent === "string" ? { parent_id: parent } : {}),
      title: typeof title === "string" ? title : "",
      modified: now,
      created_by: owner,
    };
    next += 1;
    items.push(created);
    return created;
  };

  /** Applies a change to an item and answers it. */
  const change = (
    method: string,
    item: Item,
    action: string | undefined,
    body: Record<string, unknown>,
  ) => {
    const title = body["title"];
    if (method === "PATCH" && typeof title === "string")
      item.title = title.trim();
    if (action === "star") {
      if (body["starred"] === true) starred.add(item.id);
      else starred.delete(item.id);
      return { ok: true };
    }
    if (action === "trash") items.splice(items.indexOf(item), 1);
    if (action === "move") move(item, body);
    return item;
  };

  const handle = async (route: Route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1/drive", "");
    const method = request.method();
    if (method === "GET") {
      const answer = read(path, url.searchParams);
      return route.fulfill(answer === undefined ? notFound : { json: answer });
    }
    const key = `${method} ${path}`;
    changes.push(key);
    const refusal = [...refusals].find(([suffix]) => key.endsWith(suffix));
    if (refusal !== undefined) {
      refusals.delete(refusal[0]);
      const { status, ...body } = refusal[1];
      return route.fulfill({ status, json: { ...body, retryable: false } });
    }
    const body = (request.postDataJSON() ?? {}) as Record<string, unknown>;
    if (method === "POST" && path === `/spaces/${SPACE.id}/pages`) {
      return route.fulfill({ json: create(body) });
    }
    const parts = path.split("/");
    const item = items.find((entry) => entry.id === parts[2]);
    if (item === undefined) return route.fulfill(notFound);
    return route.fulfill({ json: change(method, item, parts[3], body) });
  };
  await page.route("**/api/v1/drive/**", handle);
  // Full-text search, over titles only: enough to show the app asks it.
  await page.route("**/api/v1/search?*", (route) => {
    const q = (
      new URL(route.request().url()).searchParams.get("q") ?? ""
    ).toLowerCase();
    const hits = items
      .filter((item) => item.title.toLowerCase().includes(q))
      .map((item) => ({
        doc: item.id,
        snippet: [{ text: item.title, hit: true }],
      }));
    return route.fulfill({ json: { hits, has_more: false, hybrid: false } });
  });
  return {
    items,
    shared,
    starred,
    changes,
    refuseNext(key, refusal) {
      refusals.set(key, refusal);
    },
  };
}
