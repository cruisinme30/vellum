// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";

const ISSUES = `/data/${COLLECTION.issues}`;

describe("CollectionView", () => {
  it("shows the collection's views as tabs controlling one panel", async () => {
    renderApp(ISSUES);
    expect(screen.getByRole("status").textContent).toBe("Loading…");
    expect(
      await screen.findByRole("heading", { level: 1, name: "Issues" }),
    ).toBeDefined();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      "All issues",
      "Board",
      "Gallery",
      "Calendar",
      "Timeline",
      "Open",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expect(tabs[0]?.tabIndex).toBe(0);
    expect(tabs[1]?.tabIndex).toBe(-1);
    const panel = screen.getByRole("tabpanel");
    expect(panel.getAttribute("aria-labelledby")).toBe(tabs[0]?.id);
    expect(tabs[0]?.getAttribute("aria-controls")).toBe(panel.id);
    const crumbs = screen.getByRole("navigation", { name: "Breadcrumbs" });
    expect(
      within(crumbs).getByRole("link", { name: "Data" }).getAttribute("href"),
    ).toBe("/data");
  });

  it(
    "switches views with a click or the arrow keys, in the " + "address",
    async () => {
      const { user, location } = renderApp(ISSUES);
      await user.click(await screen.findByRole("tab", { name: "Board" }));
      expect(location()).toBe(`${ISSUES}?view=Board`);
      expect(
        await screen.findByRole("region", { name: /^Done/ }),
      ).toBeDefined();
      expect(
        screen
          .getByRole("tab", { name: "Board" })
          .getAttribute("aria-selected"),
      ).toBe("true");
      act(() => {
        screen.getByRole("tab", { name: "Board" }).focus();
      });
      await user.keyboard("{ArrowRight}");
      expect(location()).toBe(`${ISSUES}?view=Gallery`);
      expect(document.activeElement).toBe(
        screen.getByRole("tab", { name: "Gallery" }),
      );
      await user.keyboard("{End}");
      expect(location()).toBe(`${ISSUES}?view=Open`);
      await user.keyboard("{ArrowRight}");
      expect(location()).toBe(`${ISSUES}?view=All+issues`);
      await user.keyboard("{ArrowLeft}");
      expect(location()).toBe(`${ISSUES}?view=Open`);
      await user.keyboard("{Home}x");
      expect(location()).toBe(`${ISSUES}?view=All+issues`);
      expect(await screen.findByRole("grid")).toBeDefined();
    },
  );

  it("opens the view named in the address", async () => {
    renderApp(`${ISSUES}?view=timeline`);
    expect(await screen.findByRole("list", { name: "Timeline" })).toBeDefined();
    expect(
      screen
        .getByRole("tab", { name: "Timeline" })
        .getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("says when a view does not exist", async () => {
    renderApp(`${ISSUES}?view=Nope`);
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      'no view named "Nope"',
    );
  });

  it("says when the collection does not exist", async () => {
    const { user, location } = renderApp("/data/nope");
    expect(
      await screen.findByRole("heading", {
        level: 1,
        name: "This collection does not exist",
      }),
    ).toBeDefined();
    expect(screen.getByText("collection nope not found")).toBeDefined();
    await user.click(
      screen.getByRole("link", { name: "Back to all collections" }),
    );
    expect(location()).toBe("/data");
  });

  it("creates a row and puts the caret in its title", async () => {
    const collections = createMockCollectionsStore();
    const { user } = renderApp(ISSUES, undefined, collections);
    await screen.findByRole("grid");
    await user.click(screen.getByRole("button", { name: "New row" }));
    const panel = await screen.findByRole("dialog", { name: "Untitled" });
    const title = await within(panel).findByRole("textbox", { name: "Issue" });
    await waitFor(() => {
      expect(document.activeElement).toBe(title);
    });
    await user.type(title, "Shed load at the edge{Enter}");
    expect(
      await screen.findByRole("dialog", { name: "Shed load at the edge" }),
    ).toBeDefined();
    const grid = screen.getByRole("grid");
    expect(
      await within(grid).findByText("Shed load at the edge"),
    ).toBeDefined();
    expect(grid.getAttribute("aria-rowcount")).toBe("10");
  });

  it("says when a row cannot be created", async () => {
    const collections = createMockCollectionsStore();
    vi.spyOn(collections, "createRow").mockRejectedValueOnce(
      new Error("no write access"),
    );
    const { user } = renderApp(ISSUES, undefined, collections);
    await screen.findByRole("grid");
    await user.click(screen.getByRole("button", { name: "New row" }));
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "no write access",
    );
    vi.spyOn(collections, "createRow").mockResolvedValueOnce({
      changed: { written: [], deleted: [] },
    });
    await user.click(screen.getByRole("button", { name: "New row" }));
    await waitFor(() => {
      expect(screen.queryByRole("alert")).toBeNull();
    });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
