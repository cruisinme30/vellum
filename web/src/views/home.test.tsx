// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PAGE } from "../data/mock/northstar";
import type { WorkspaceStore } from "../data/store";
import { renderWithProviders } from "../test/render-app";

import { FavoritesView, HomeView } from "./home";

const section = (name: string) => screen.getByRole("region", { name });
const linkNames = (element: HTMLElement) =>
  within(element)
    .queryAllByRole("link")
    .map((link) => link.textContent);

function emptyWorkspace(store: WorkspaceStore) {
  act(() => {
    for (const page of store.getSnapshot().pages.filter((p) => p.parentId === undefined)) {
      store.trashPage(page.id);
    }
  });
}

describe("HomeView", () => {
  it("greets the user by first name", () => {
    renderWithProviders(<HomeView />);
    expect(screen.getByRole("heading", { level: 1, name: "Good to see you, Priya" })).toBeDefined();
  });

  it("lists the six most recently changed pages, newest first", () => {
    const { store } = renderWithProviders(<HomeView />);
    expect(linkNames(section("Recently changed"))).toHaveLength(6);
    act(() => {
      store.renamePage(PAGE.directory, "Team directory 2027");
    });
    const recent = section("Recently changed");
    expect(within(recent).getAllByRole("link")[0]).toBe(
      within(recent).getByRole("link", { name: "Team directory 2027" }),
    );
  });

  it("lists favourites and links each page", () => {
    renderWithProviders(<HomeView />);
    const favorites = section("Favorites");
    expect(linkNames(favorites)).toHaveLength(3);
    for (const name of ["Engineering home", "Platform roadmap 2027", "PRD: Policy simulator"]) {
      expect(within(favorites).getByRole("link", { name })).toBeDefined();
    }
    expect(
      within(section("Favorites"))
        .getByRole("link", { name: "Engineering home" })
        .getAttribute("href"),
    ).toBe(`/p/${PAGE.engHome}`);
  });

  it("says when there is nothing to show", () => {
    const { store } = renderWithProviders(<HomeView />);
    emptyWorkspace(store);
    expect(within(section("Recently changed")).getByText("Nothing has changed yet.")).toBeDefined();
    expect(within(section("Favorites")).getByText("Star a page to keep it here.")).toBeDefined();
  });
});

describe("FavoritesView", () => {
  it("lists starred pages and follows changes", () => {
    const { store } = renderWithProviders(<FavoritesView />);
    expect(screen.getByRole("heading", { level: 1, name: "Favorites" })).toBeDefined();
    expect(linkNames(document.body)).toHaveLength(3);
    act(() => {
      store.toggleStar(PAGE.launch);
    });
    expect(screen.getByRole("link", { name: "Launch checklist" })).toBeDefined();
  });

  it("says how to add a favourite", () => {
    const { store } = renderWithProviders(<FavoritesView />);
    emptyWorkspace(store);
    expect(screen.getByText("Star a page to keep it here.")).toBeDefined();
  });
});
