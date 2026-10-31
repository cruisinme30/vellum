// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderWithProviders } from "../test/render-app";

import { Sidebar } from "./sidebar";

const HANDBOOK = "01JB3K0000SPACE0HBK0000003";

function renderSidebar(compact: boolean) {
  const onNavigate = vi.fn<() => void>();
  return {
    ...renderWithProviders(
      <Sidebar
        currentId={undefined}
        compact={compact}
        onNavigate={onNavigate}
      />,
    ),
    onNavigate,
  };
}

describe("Sidebar", () => {
  it("shows the workspace and one page tree per space", () => {
    renderSidebar(false);
    expect(screen.getByText("Northstar")).toBeDefined();
    expect(
      screen
        .getAllByRole("heading", { level: 2 })
        .map((heading) => heading.textContent),
    ).toEqual(["🛰 Engineering", "📐 Product", "📘 Handbook"]);
    expect(
      screen
        .getAllByRole("tree")
        .map((tree) => tree.getAttribute("aria-label")),
    ).toEqual(["Engineering pages", "Product pages", "Handbook pages"]);
    expect(screen.getByRole("region", { name: "Engineering" })).toBeDefined();
    expect(screen.queryByRole("navigation")).toBeNull();
    expect(screen.queryByRole("radio")).toBeNull();
  });

  it("adds a page to a space and opens it", async () => {
    const { user, store, location, onNavigate } = renderSidebar(false);
    await user.click(
      screen.getByRole("button", { name: "Add a page to Handbook" }),
    );
    const created = store.getSnapshot().pages.at(-1);
    expect(created).toMatchObject({ spaceId: HANDBOOK, parentId: undefined });
    await waitFor(() => {
      expect(location()).toBe(`/p/${created?.id ?? ""}`);
    });
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(screen.getByRole("treeitem", { name: "Untitled" })).toBeDefined();
  });

  it("lists the other destinations and theme settings on phones", async () => {
    const { user, location, onNavigate } = renderSidebar(true);
    await user.click(screen.getByRole("link", { name: "Favorites" }));
    expect(location()).toBe("/favorites");
    expect(onNavigate).toHaveBeenCalledOnce();
    expect(screen.getByRole("radio", { name: "Sepia" })).toBeDefined();
  });
});
