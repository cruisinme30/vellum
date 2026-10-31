// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** A fake workspace server and the data the HTTP store tests load from it. */

import { expect, vi } from "vitest";

import { ApiError, type Http } from "../api/http";
import { createHttpWorkspaceStore } from "../data/http";
import type { WorkspaceStore } from "../data/store";

export const PRIYA = {
  id: "01M6409GN2EYWS8Z7NFQQ3JNWZ",
  handle: "priya",
  name: "Priya Raman",
};
export const SAM = {
  id: "01M6409GN2EYWS8Z7NFQQ3SAMS",
  handle: "sam",
  name: "Sam Lee",
};
const MINE = {
  id: "S-MINE",
  kind: "user_drive",
  slug: "priya",
  name: "My Drive",
};
const TEAM = {
  id: "S-TEAM",
  kind: "shared_drive",
  slug: "eng",
  name: "Engineering",
};
export const T = "2026-10-29T10:00:00Z";

export function node(
  id: string,
  kind: string,
  parent?: string,
  extra: Record<string, unknown> = {},
) {
  return {
    id,
    space_id: "S-MINE",
    kind,
    parent_id: parent,
    title: id,
    path: `drives/users/priya/${id}`,
    size: 1,
    modified: T,
    created_by: PRIYA.id,
    ...extra,
  };
}

type Route = (body: unknown, query: unknown) => unknown;

/**
 * A fake server: GET and send answers by "METHOD path"; an Error answer
 * rejects.
 */
export function fakeHttp(routes: Record<string, unknown>) {
  const answer = (
    key: string,
    body: unknown,
    query?: unknown,
  ): Promise<unknown> => {
    if (!(key in routes))
      return Promise.reject(new ApiError("not_found", `no route ${key}`));
    const route = routes[key];
    const value: unknown =
      typeof route === "function" ? (route as Route)(body, query) : route;
    return value instanceof Error
      ? Promise.reject(value)
      : Promise.resolve(value);
  };
  const http = {
    get: vi.fn((path: string, query?: Record<string, unknown>) => {
      const parent = query?.["parent_id"];
      const key =
        typeof parent === "string"
          ? `GET ${path}?parent_id=${parent}`
          : `GET ${path}`;
      return answer(key, undefined, query);
    }),
    send: vi.fn((method: string, path: string, body?: unknown) =>
      answer(`${method} ${path}`, body),
    ),
    forgetSession: vi.fn(),
  };
  return http as typeof http & Http;
}

export const BASE = {
  "GET /api/v1/drive/my-drive": { space: MINE },
  "GET /api/v1/drive/shared-drives": { spaces: [TEAM] },
  "GET /api/v1/drive/starred": { items: [node("p2", "page")] },
  "GET /api/v1/drive/spaces/S-MINE/tree": {
    items: [
      node("f1", "folder"),
      node("p2", "page", "f1", { created_by: SAM.id }),
      node("p1", "page"),
      node("b1", "book"),
      node("in-book", "page", "b1"),
    ],
    truncated: false,
  },
  "GET /api/v1/drive/spaces/S-TEAM/tree": {
    items: [node("t1", "page", undefined, { space_id: "S-TEAM" })],
    truncated: false,
  },
};

/** A loaded store over the base server plus overrides. */
export async function loaded(routes: Record<string, unknown> = {}) {
  const http = fakeHttp({ ...BASE, ...routes });
  const store = createHttpWorkspaceStore(http, { user: PRIYA });
  await settled(store);
  return { http, store };
}

/**
 * Waits until the store has stopped loading and pending changes have settled.
 */
export async function settled(store: WorkspaceStore) {
  await vi.waitFor(() => {
    expect(store.getSnapshot().loading).toBe(false);
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
}

/** A search hit on `pageId` whose excerpt is "the " plus `text`. */
export const searchHit = (pageId: string, text: string) => ({
  doc: pageId,
  snippet: [
    { text: "the ", hit: false },
    { text, hit: true },
  ],
});

/** The snippet `searchHit` makes, as the store keeps it. */
export const marked = (text: string) => [
  { text: "the ", hit: false },
  { text, hit: true },
];

export const page = (store: WorkspaceStore, id: string) =>
  store.getSnapshot().pages.find((p) => p.id === id);
