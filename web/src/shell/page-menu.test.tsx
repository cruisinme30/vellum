// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PAGE } from "../data/mock/northstar";
import { renderWithProviders } from "../test/render-app";

import { Sidebar } from "./sidebar";

function renderMenu() {
  const rendered = renderWithProviders(
    <Sidebar currentId={undefined} compact={false} onNavigate={vi.fn<() => void>()} />,
    "/help",
  );
  const tree = screen.getByRole("tree", { name: "Engineering pages" });
  const openMenu = async (title: string) => {
    await rendered.user.click(within(tree).getByRole("button", { name: `Actions for ${title}` }));
    return screen.getByRole("menu");
  };
  const choose = async (title: string, action: string) => {
    const menu = await openMenu(title);
    await rendered.user.click(within(menu).getByRole("menuitem", { name: action }));
  };
  const page = (id: string) => rendered.store.getSnapshot().pages.find((p) => p.id === id);
  return { ...rendered, tree, openMenu, choose, page };
}

describe("PageMenu", () => {
  it("is a menu button that focuses its first item", async () => {
    const { user, openMenu } = renderMenu();
    const button = screen.getByRole("button", { name: "Actions for On-call handbook" });
    expect(button.getAttribute("aria-haspopup")).toBe("menu");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    const menu = await openMenu("On-call handbook");
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(button.getAttribute("aria-controls")).toBe(menu.id);
    expect(
      within(menu)
        .getAllByRole("menuitem")
        .map((item) => item.textContent),
    ).toEqual([
      "Add subpage",
      "Rename",
      "Add to favorites",
      "Move up",
      "Move down",
      "Nest under the page above",
      "Move out one level",
      "Move to trash",
    ]);
    expect(document.activeElement?.textContent).toBe("Add subpage");
    await user.click(button);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("moves through its items with the arrows, Home and End, wrapping around", async () => {
    const { user, openMenu } = renderMenu();
    await openMenu("On-call handbook");
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement?.textContent).toBe("Move to trash");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement?.textContent).toBe("Add subpage");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement?.textContent).toBe("Rename");
    await user.keyboard("{End}");
    expect(document.activeElement?.textContent).toBe("Move to trash");
    await user.keyboard("{Home}");
    expect(document.activeElement?.textContent).toBe("Add subpage");
    await user.keyboard("x");
    expect(screen.getByRole("menu")).toBeDefined();
  });

  it.each(["{Escape}", "{Tab}"])("closes on %s and focuses the row", async (key) => {
    const { user, openMenu } = renderMenu();
    await openMenu("On-call handbook");
    await user.keyboard(key);
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("treeitem", { name: "On-call handbook" }));
  });

  it("stars and unstars a page with Enter and Space", async () => {
    const { user, openMenu, page } = renderMenu();
    await openMenu("On-call handbook");
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(page(PAGE.onCall)?.starred).toBe(true);
    expect(screen.queryByRole("menu")).toBeNull();
    const menu = await openMenu("On-call handbook");
    const unstar = within(menu).getByRole("menuitem", { name: "Remove from favorites" });
    unstar.focus();
    await user.keyboard(" ");
    expect(page(PAGE.onCall)?.starred).toBe(false);
  });

  it("starts renaming the page", async () => {
    const { user, choose, page } = renderMenu();
    await choose("On-call handbook", "Rename");
    const field = screen.getByRole("textbox", { name: "Page title" });
    await user.clear(field);
    await user.type(field, "Pager guide{Enter}");
    expect(page(PAGE.onCall)?.title).toBe("Pager guide");
  });

  it("moves the page up, down, in and out", async () => {
    const { choose, page } = renderMenu();
    await choose("On-call handbook", "Move up");
    expect(page(PAGE.onCall)?.position).toBe(1);
    await choose("On-call handbook", "Move down");
    expect(page(PAGE.onCall)?.position).toBe(2);
    await choose("On-call handbook", "Nest under the page above");
    expect(page(PAGE.onCall)?.parentId).toBe(PAGE.gateway);
    expect(screen.getByRole("treeitem", { name: "On-call handbook" })).toBeDefined();
    await choose("On-call handbook", "Move out one level");
    expect(page(PAGE.onCall)?.parentId).toBe(PAGE.engHome);
  });

  it("adds a subpage and opens it", async () => {
    const { choose, store, location } = renderMenu();
    await choose("Platform roadmap 2027", "Add subpage");
    const child = store.getSnapshot().pages.find((p) => p.parentId === PAGE.roadmap);
    expect(child).toBeDefined();
    expect(location()).toBe(`/p/${child?.id ?? ""}`);
    expect(screen.getByRole("treeitem", { name: "Platform roadmap 2027" })).toHaveProperty(
      "ariaExpanded",
      "true",
    );
    expect(screen.getByRole("button", { name: "Actions for Untitled" })).toBeDefined();
  });

  it("moves the page and its subpages to the trash and goes home", async () => {
    const { choose, page, location } = renderMenu();
    await choose("Gateway architecture", "Move to trash");
    expect(page(PAGE.gateway)).toBeUndefined();
    expect(page(PAGE.runbook)).toBeUndefined();
    await waitFor(() => {
      expect(location()).toBe("/");
    });
  });
});
