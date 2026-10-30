// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AuthApi, AuthStatus } from "../api/auth";
import { ApiError, type Http } from "../api/http";
import { createMockCollectionsStore } from "../collections/mock";
import { createHttpWorkspaceStore } from "../data/http";
import { renderApp } from "../test/render-app";

// The pages screens against a server's drive, through the HTTP store.

const USER = {
  id: "01M6409GN2EYWS8Z7NFQQ3JNWZ",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: false,
};
const STATUS: AuthStatus = { needsSetup: false, credential: "session", scopes: [], user: USER };

function node(id: string, kind: "page" | "folder", parent?: string) {
  return {
    id,
    space_id: "S1",
    kind,
    parent_id: parent,
    title: id === "F1" ? "Plans" : `Page ${id}`,
    modified: "2026-10-29T10:00:00Z",
    created_by: USER.id,
  };
}

/** A drive with one folder holding one page; `hold` keeps the tree from arriving. */
function fakeDrive(options: { hold?: boolean; trash?: unknown } = {}) {
  const routes: Record<string, unknown> = {
    "/api/v1/drive/my-drive": {
      space: { id: "S1", kind: "user_drive", slug: "priya", name: "Mine" },
    },
    "/api/v1/drive/shared-drives": { spaces: [] },
    "/api/v1/drive/starred": { items: [] },
    "/api/v1/drive/spaces/S1/tree": {
      items: [node("F1", "folder"), node("P1", "page", "F1")],
      truncated: false,
    },
    // Shared with Priya on its own, from a drive she cannot browse.
    "/api/v1/drive/items/X1": {
      ...node("X1", "page", "XF"),
      space_id: "S9",
      title: "Launch plan Q1",
      created_by: "SOMEONE",
    },
  };
  const http = {
    get: vi.fn((path: string, query?: { parent_id?: string }) => {
      if (options.hold === true)
        return new Promise<never>(() => {
          // Never answers.
        });
      const key = query?.parent_id === undefined ? path : `${path}?${query.parent_id}`;
      return key in routes
        ? Promise.resolve(routes[key])
        : Promise.reject(new ApiError("not_found", "no such item"));
    }),
    send: vi.fn(() =>
      options.trash instanceof Error ? Promise.reject(options.trash) : Promise.resolve({}),
    ),
    forgetSession: vi.fn(),
  };
  return http as typeof http & Http;
}

function renderServer(path: string, http: Http) {
  const store = createHttpWorkspaceStore(http);
  const auth: AuthApi = {
    status: () => Promise.resolve(STATUS),
    signIn: () => Promise.resolve(USER),
    signOut: () => Promise.resolve(),
    setUp: () => Promise.resolve(USER),
  };
  return renderApp(path, store, createMockCollectionsStore(), { auth, status: STATUS });
}

describe("pages on a server", () => {
  it("loads the signed-in person's drive into the sidebar", async () => {
    renderServer("/", fakeDrive());
    expect(await screen.findByRole("treeitem", { name: "Plans" })).toBeDefined();
    expect(screen.getByRole("heading", { name: /Mine/ })).toBeDefined();
  });

  it("says the pages are loading rather than that a page is missing", async () => {
    renderServer("/p/P1", fakeDrive({ hold: true }));
    expect(await screen.findByText("Loading your pages…", {}, { timeout: 5000 })).toBeDefined();
    expect(screen.queryByText("This page does not exist")).toBeNull();
  });

  it("opens a page shared on its own, outside every space the person can browse", async () => {
    // #567: it was looked up only in the spaces' trees, so it "did not exist".
    renderServer("/p/X1", fakeDrive());
    const title = await screen.findByRole("textbox", { name: "Page title" });
    expect(title).toHaveProperty("value", "Launch plan Q1");
    expect(screen.queryByText("This page does not exist")).toBeNull();
    // It does not join the sidebar's trees.
    expect(screen.queryByRole("treeitem", { name: "Launch plan Q1" })).toBeNull();
  });

  it("says a page does not exist when the server has none by that ID", async () => {
    renderServer("/p/NOPE", fakeDrive());
    expect(
      await screen.findByRole("heading", { level: 1, name: "This page does not exist" }),
    ).toBeDefined();
  });

  it("shows a folder as the list of pages in it", async () => {
    renderServer("/p/F1", fakeDrive());
    const title = await screen.findByRole("textbox", { name: "Page title" });
    expect(title).toHaveProperty("value", "Plans");
    expect(screen.getByText("A folder. The pages in it:")).toBeDefined();
    const main = screen.getByRole("main");
    expect(within(main).getByRole("link", { name: /Page P1/ })).toBeDefined();
  });

  it("shows a refused change with its request ID until dismissed", async () => {
    const http = fakeDrive({
      trash: new ApiError("permission_denied", "You may only view this.", { requestId: "R9" }),
    });
    const { user } = renderServer("/", http);
    await user.click(await screen.findByRole("button", { name: "Actions for Plans" }));
    await user.click(screen.getByRole("menuitem", { name: "Move to trash" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("You may only view this. (request R9)");
    expect(screen.getByRole("treeitem", { name: "Plans" })).toBeDefined();
    await user.click(within(alert).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
