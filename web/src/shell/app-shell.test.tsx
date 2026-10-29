// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PAGE } from "../data/mock/northstar";
import { matchQueries, renderApp } from "../test/render-app";

import { MEDIUM } from "./use-media";

const PRODUCT = "01JB3K0000SPACE0PRD0000002";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("AppShell", () => {
  it("has a skip link to the main area", () => {
    renderApp("/");
    const skip = screen.getByRole("link", { name: "Skip to content" });
    expect(skip.getAttribute("href")).toBe(`#${screen.getByRole("main").id}`);
  });

  it("keeps the sidebar open on wide windows when a page opens", async () => {
    const { user, location } = renderApp("/help");
    await user.click(screen.getByRole("treeitem", { name: "Launch checklist" }));
    expect(location()).toBe(`/p/${PAGE.launch}`);
    expect(screen.getByRole("complementary", { name: "Pages" })).toBeDefined();
    expect(
      screen.getByRole("treeitem", { name: "Launch checklist" }).getAttribute("aria-current"),
    ).toBe("page");
  });

  it("creates a page in the space of the open page", async () => {
    const { user, store, location } = renderApp(`/p/${PAGE.launch}`);
    await user.click(screen.getByRole("button", { name: "New" }));
    const created = store.getSnapshot().pages.at(-1);
    expect(created).toMatchObject({ spaceId: PRODUCT, parentId: undefined, title: "" });
    await waitFor(() => {
      expect(location()).toBe(`/p/${created?.id ?? ""}`);
    });
  });

  it("creates a page in the first space elsewhere", async () => {
    const { user, store } = renderApp("/help");
    await user.click(screen.getByRole("button", { name: "New" }));
    expect(store.getSnapshot().pages.at(-1)?.spaceId).toBe("01JB3K0000SPACE0ENG0000001");
  });
});

describe("AppShell on medium windows", () => {
  it("starts with the sidebar closed and opens it over a scrim", async () => {
    vi.stubGlobal("matchMedia", matchQueries(MEDIUM));
    const { user, container } = renderApp("/");
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
    expect(container.querySelector(".vl-scrim")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Show pages" }));
    expect(screen.getByRole("complementary", { name: "Pages" })).toBeDefined();
    const scrim = container.querySelector(".vl-scrim");
    expect(scrim).not.toBeNull();
    if (scrim) {
      await user.click(scrim);
    }
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
  });

  it("closes the sidebar after opening a page from it", async () => {
    vi.stubGlobal("matchMedia", matchQueries(MEDIUM));
    const { user, location } = renderApp("/help");
    await user.click(screen.getByRole("button", { name: "Show pages" }));
    const tree = screen.getByRole("tree", { name: "Handbook pages" });
    await user.click(within(tree).getByRole("treeitem", { name: "Team directory" }));
    expect(location()).toBe(`/p/${PAGE.directory}`);
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
  });
});
