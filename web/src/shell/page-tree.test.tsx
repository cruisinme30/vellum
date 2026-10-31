// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  act,
  createEvent,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { NORTHSTAR, PAGE } from "../data/mock/northstar";
import { buildTree } from "../data/tree";
import { renderWithProviders } from "../test/render-app";

import { dropTarget, visibleRows } from "./page-tree";
import { Sidebar } from "./sidebar";

const ENGINEERING = "01JB3K0000SPACE0ENG0000001";

function renderTree(currentId?: string) {
  const onNavigate = vi.fn<() => void>();
  const rendered = renderWithProviders(
    <Sidebar currentId={currentId} compact={false} onNavigate={onNavigate} />,
  );
  const tree = screen.getByRole("tree", { name: "Engineering pages" });
  const item = (title: string) =>
    within(tree).getByRole("treeitem", { name: title });
  const titles = () =>
    within(tree)
      .getAllByRole("treeitem")
      .map((row) => row.querySelector(".vl-tree-title")?.textContent);
  const focusOn = (title: string) => {
    act(() => {
      item(title).focus();
    });
  };
  const titleOf = (id: string) =>
    rendered.store.getSnapshot().pages.find((p) => p.id === id);
  return { ...rendered, onNavigate, tree, item, titles, focusOn, titleOf };
}

describe("visibleRows", () => {
  const tree = buildTree(NORTHSTAR.pages, ENGINEERING);

  it("lists top-level pages only when nothing is expanded", () => {
    const rows = visibleRows(tree, new Set());
    expect(rows.map((row) => row.node.page.id)).toEqual([
      PAGE.engHome,
      PAGE.roadmap,
    ]);
    expect(rows[1]).toMatchObject({
      level: 1,
      index: 1,
      size: 2,
      parentId: undefined,
    });
  });

  it("walks into expanded pages with their level and parent", () => {
    const rows = visibleRows(tree, new Set([PAGE.engHome, PAGE.gateway]));
    expect(
      rows.map((row) => [row.node.page.id, row.level, row.parentId]),
    ).toEqual([
      [PAGE.engHome, 1, undefined],
      [PAGE.gateway, 2, PAGE.engHome],
      [PAGE.rateLimits, 3, PAGE.gateway],
      [PAGE.runbook, 3, PAGE.gateway],
      [PAGE.onCall, 2, PAGE.engHome],
      [PAGE.roadmap, 1, undefined],
    ]);
  });
});

describe("dropTarget", () => {
  const rows = visibleRows(
    buildTree(NORTHSTAR.pages, ENGINEERING),
    new Set([PAGE.engHome]),
  );
  const gateway = rows[1];

  it("drops before, inside or after a row by pointer height", () => {
    expect(gateway).toBeDefined();
    if (!gateway) {
      return;
    }
    expect(dropTarget(gateway, "space", 5, 40)).toEqual({
      spaceId: "space",
      parentId: PAGE.engHome,
      index: 0,
    });
    expect(dropTarget(gateway, "space", 20, 40)).toEqual({
      spaceId: "space",
      parentId: PAGE.gateway,
      index: 2,
    });
    expect(dropTarget(gateway, "space", 35, 40)).toEqual({
      spaceId: "space",
      parentId: PAGE.engHome,
      index: 1,
    });
  });
});

describe("PageTree", () => {
  it("is a tree with levels, positions and the current page selected", () => {
    const { item, titles } = renderTree(PAGE.rateLimits);
    expect(titles()).toEqual([
      "Engineering home",
      "Gateway architecture",
      "Rate limiting design",
      "Runbook: deploy the gateway",
      "On-call handbook",
      "Platform roadmap 2027",
    ]);
    const current = item("Rate limiting design");
    expect(current.getAttribute("aria-level")).toBe("3");
    expect(current.getAttribute("aria-posinset")).toBe("1");
    expect(current.getAttribute("aria-setsize")).toBe("2");
    expect(current.getAttribute("aria-selected")).toBe("true");
    expect(current.getAttribute("aria-current")).toBe("page");
    expect(current.tabIndex).toBe(0);
    expect(item("Engineering home").tabIndex).toBe(-1);
    expect(item("Gateway architecture").getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(item("On-call handbook").hasAttribute("aria-expanded")).toBe(false);
  });

  it("moves focus with the arrows, Home and End", async () => {
    const { user, item, focusOn } = renderTree();
    expect(item("Engineering home").tabIndex).toBe(0);
    focusOn("Engineering home");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(item("Gateway architecture"));
    expect(item("Gateway architecture").tabIndex).toBe(0);
    await user.keyboard("{End}");
    expect(document.activeElement).toBe(item("Platform roadmap 2027"));
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(item("Platform roadmap 2027"));
    await user.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(item("On-call handbook"));
    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(item("Engineering home"));
  });

  it("expands, enters, leaves and collapses with Right and Left", async () => {
    const { user, item, focusOn, titles } = renderTree();
    focusOn("Gateway architecture");
    await user.keyboard("{ArrowRight}");
    expect(item("Gateway architecture").getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(titles()).toContain("Runbook: deploy the gateway");
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(item("Rate limiting design"));
    await user.keyboard("{ArrowLeft}");
    expect(document.activeElement).toBe(item("Gateway architecture"));
    await user.keyboard("{ArrowLeft}");
    expect(item("Gateway architecture").getAttribute("aria-expanded")).toBe(
      "false",
    );
    expect(titles()).not.toContain("Rate limiting design");
    focusOn("Platform roadmap 2027");
    await user.keyboard("{ArrowLeft}{ArrowRight}");
    expect(document.activeElement).toBe(item("Platform roadmap 2027"));
  });

  it("toggles a page with its twisty without opening it", async () => {
    const { user, location } = renderTree();
    await user.click(
      screen.getByRole("button", { name: "Expand Gateway architecture" }),
    );
    expect(
      screen.getByRole("button", { name: "Collapse Gateway architecture" }),
    ).toBeDefined();
    expect(location()).toBe("/");
  });

  it("opens a page with Enter or a click", async () => {
    const { user, item, focusOn, location, onNavigate } = renderTree();
    focusOn("On-call handbook");
    await user.keyboard("{Enter}");
    expect(location()).toBe(`/p/${PAGE.onCall}`);
    await user.click(item("Platform roadmap 2027"));
    expect(location()).toBe(`/p/${PAGE.roadmap}`);
    expect(onNavigate).toHaveBeenCalledTimes(2);
  });

  it(
    "renames with F2: Enter saves, Escape cancels, leaving the field " +
      "saves",
    async () => {
      const { user, item, focusOn, titleOf } = renderTree();
      focusOn("On-call handbook");
      await user.keyboard("{F2}");
      const field = screen.getByRole("textbox", { name: "Page title" });
      expect(document.activeElement).toBe(field);
      await user.click(field);
      await user.clear(field);
      await user.keyboard("  Pager guide  {Enter}");
      expect(titleOf(PAGE.onCall)?.title).toBe("Pager guide");
      expect(document.activeElement).toBe(item("Pager guide"));

      await user.keyboard("{F2}");
      await user.keyboard("Ignored{Escape}");
      expect(titleOf(PAGE.onCall)?.title).toBe("Pager guide");
      expect(screen.queryByRole("textbox")).toBeNull();

      await user.keyboard("{F2}");
      await user.keyboard("{End} v2");
      await user.tab();
      expect(titleOf(PAGE.onCall)?.title).toBe("Pager guide v2");
    },
  );

  it("shows untitled pages by a placeholder name", () => {
    const { store, item } = renderTree();
    act(() => {
      store.renamePage(PAGE.roadmap, "");
    });
    expect(item("Untitled").textContent).toContain("Untitled");
  });

  it("reorders, indents and outdents with Alt+Shift+arrows", async () => {
    const { user, item, focusOn, titles, titleOf } = renderTree();
    focusOn("Platform roadmap 2027");
    await user.keyboard("{Alt>}{Shift>}{ArrowUp}{/Shift}{/Alt}");
    expect(titles()[0]).toBe("Platform roadmap 2027");
    await waitFor(() => {
      expect(document.activeElement).toBe(item("Platform roadmap 2027"));
    });
    await user.keyboard("{Alt>}{Shift>}{ArrowUp}{/Shift}{/Alt}");
    expect(titles()[0]).toBe("Platform roadmap 2027");
    await user.keyboard("{Alt>}{Shift>}{ArrowDown}{/Shift}{/Alt}");
    expect(titles().at(-1)).toBe("Platform roadmap 2027");
    await waitFor(() => {
      expect(document.activeElement).toBe(item("Platform roadmap 2027"));
    });
    await user.keyboard("{Alt>}{Shift>}{ArrowRight}{/Shift}{/Alt}");
    expect(titleOf(PAGE.roadmap)?.parentId).toBe(PAGE.engHome);
    expect(item("Platform roadmap 2027").getAttribute("aria-level")).toBe("2");
    await waitFor(() => {
      expect(document.activeElement).toBe(item("Platform roadmap 2027"));
    });
    await user.keyboard("{Alt>}{Shift>}{ArrowLeft}{/Shift}{/Alt}");
    expect(titleOf(PAGE.roadmap)?.parentId).toBeUndefined();
  });

  it("opens the page menu with Shift+F10", async () => {
    const { user, focusOn } = renderTree();
    focusOn("On-call handbook");
    await user.keyboard("{Shift>}{F10}{/Shift}");
    const menu = screen.getByRole("menu");
    expect(document.activeElement).toBe(
      within(menu).getAllByRole("menuitem")[0],
    );
  });
});

function dataTransfer() {
  const data = new Map<string, string>();
  return {
    effectAllowed: "none",
    setData: (type: string, value: string) => data.set(type, value),
    getData: (type: string) => data.get(type) ?? "",
  };
}

function drop(target: HTMLElement, clientY: number, transfer = dataTransfer()) {
  vi.spyOn(target, "getBoundingClientRect").mockReturnValue(
    new DOMRect(0, 0, 200, 40),
  );
  fireEvent.dragOver(target, { dataTransfer: transfer });
  // jsdom has no DragEvent, so the pointer position is set by hand.
  const event = createEvent.drop(target, { dataTransfer: transfer });
  Object.defineProperty(event, "clientY", { value: clientY });
  fireEvent(target, event);
}

describe("PageTree drag and drop", () => {
  it("drops a page inside another row and expands it", () => {
    const { item, titleOf } = renderTree();
    const transfer = dataTransfer();
    fireEvent.dragStart(item("Platform roadmap 2027"), {
      dataTransfer: transfer,
    });
    expect(transfer.getData("application/x-vellum-page")).toBe(PAGE.roadmap);
    expect(transfer.effectAllowed).toBe("move");
    drop(item("On-call handbook"), 20, transfer);
    expect(titleOf(PAGE.roadmap)?.parentId).toBe(PAGE.onCall);
    expect(item("On-call handbook").getAttribute("aria-expanded")).toBe("true");
  });

  it("drops a page before or after a row", () => {
    const { item, titles } = renderTree();
    fireEvent.dragStart(item("Platform roadmap 2027"), {
      dataTransfer: dataTransfer(),
    });
    drop(item("Engineering home"), 2);
    expect(titles()[0]).toBe("Platform roadmap 2027");
    fireEvent.dragStart(item("Platform roadmap 2027"), {
      dataTransfer: dataTransfer(),
    });
    drop(item("Engineering home"), 38);
    expect(titles().at(-1)).toBe("Platform roadmap 2027");
  });

  it(
    "accepts a page dragged from elsewhere and ignores empty or self " +
      "drops",
    () => {
      const { item, titles, titleOf } = renderTree();
      const before = titles();
      drop(item("Engineering home"), 20);
      expect(titles()).toEqual(before);
      fireEvent.dragStart(item("On-call handbook"), {
        dataTransfer: dataTransfer(),
      });
      drop(item("On-call handbook"), 20);
      expect(titleOf(PAGE.onCall)?.parentId).toBe(PAGE.engHome);
      const foreign = dataTransfer();
      foreign.setData("application/x-vellum-page", PAGE.launch);
      drop(item("Platform roadmap 2027"), 38, foreign);
      expect(titleOf(PAGE.launch)?.spaceId).toBe(ENGINEERING);
      expect(titles().at(-1)).toBe("Launch checklist");
    },
  );
});
