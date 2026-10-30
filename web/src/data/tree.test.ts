// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  ancestors,
  buildTree,
  descendants,
  movePage,
  siblings,
  targetFor,
  type TreeNode,
} from "./tree";
import type { PageMeta } from "./types";

const SPACE = "space-a";
const OTHER_SPACE = "space-b";
/** The parent id of a top-level page. */
const TOP_LEVEL = undefined;
const alphabetical = (a: string, b: string) => a.localeCompare(b);

function page(
  id: string,
  parentId: string | undefined,
  position: number,
  spaceId = SPACE,
  title = id,
): PageMeta {
  return {
    id,
    spaceId,
    parentId,
    position,
    title,
    icon: "📄",
    status: "draft",
    owner: "priya",
    updatedAt: "2026-10-01T00:00:00.000Z",
    starred: false,
  };
}

/**
 * root-1 > (child-1 > grandchild), child-2; root-2; and one page in another
 * space.
 */
const PAGES: readonly PageMeta[] = [
  page("root-2", undefined, 2),
  page("child-2", "root-1", 2),
  page("root-1", undefined, 1),
  page("child-1", "root-1", 1),
  page("grandchild", "child-1", 1),
  page("elsewhere", undefined, 1, OTHER_SPACE),
];

const ids = (pages: readonly PageMeta[]) => pages.map((entry) => entry.id);
const shape = (nodes: readonly TreeNode[]): unknown[] =>
  nodes.map((node) =>
    node.children.length === 0
      ? node.page.id
      : [node.page.id, shape(node.children)],
  );
const byId = (pages: readonly PageMeta[], id: string) =>
  pages.find((entry) => entry.id === id);

describe("siblings", () => {
  it("lists the pages under one parent of one space, by position", () => {
    expect(ids(siblings(PAGES, SPACE, TOP_LEVEL))).toEqual([
      "root-1",
      "root-2",
    ]);
    expect(ids(siblings(PAGES, SPACE, "root-1"))).toEqual([
      "child-1",
      "child-2",
    ]);
    expect(ids(siblings(PAGES, OTHER_SPACE, TOP_LEVEL))).toEqual(["elsewhere"]);
  });

  it("breaks position ties by title", () => {
    const pages = [
      page("b", undefined, 1, SPACE, "Beta"),
      page("a", undefined, 1, SPACE, "Alpha"),
    ];
    expect(ids(siblings(pages, SPACE, TOP_LEVEL))).toEqual(["a", "b"]);
  });
});

describe("buildTree", () => {
  it("nests the pages of one space", () => {
    expect(shape(buildTree(PAGES, SPACE))).toEqual([
      ["root-1", [["child-1", ["grandchild"]], "child-2"]],
      "root-2",
    ]);
  });

  it("shows orphans at the top level", () => {
    const pages = [page("orphan", "missing", 1), page("root", undefined, 2)];
    expect(shape(buildTree(pages, SPACE))).toEqual(["orphan", "root"]);
  });

  it("stops at a cycle instead of recursing forever", () => {
    const pages = [
      page("top", undefined, 1),
      page("a", "b", 1),
      page("b", "a", 1),
    ];
    expect(shape(buildTree(pages, SPACE))).toEqual(["top"]);
  });

  it("returns an empty tree for an empty space", () => {
    expect(buildTree(PAGES, "nowhere")).toEqual([]);
  });
});

describe("descendants", () => {
  it("includes the page and everything under it", () => {
    expect([...descendants(PAGES, "root-1")].toSorted(alphabetical)).toEqual([
      "child-1",
      "child-2",
      "grandchild",
      "root-1",
    ]);
  });

  it("is just the page for a leaf", () => {
    expect([...descendants(PAGES, "grandchild")]).toEqual(["grandchild"]);
  });
});

describe("ancestors", () => {
  it("lists the chain root first", () => {
    expect(ids(ancestors(PAGES, "grandchild"))).toEqual(["root-1", "child-1"]);
  });

  it("is empty for a top-level page or an unknown id", () => {
    expect(ancestors(PAGES, "root-1")).toEqual([]);
    expect(ancestors(PAGES, "unknown")).toEqual([]);
  });

  it("skips a missing parent and survives cycles", () => {
    expect(ancestors([page("orphan", "missing", 1)], "orphan")).toEqual([]);
    const cycle = [page("a", "b", 1), page("b", "a", 1)];
    expect(ids(ancestors(cycle, "a")).toSorted(alphabetical)).toEqual([
      "a",
      "b",
    ]);
  });
});

describe("movePage", () => {
  it("reorders a page among its siblings", () => {
    const moved = movePage(PAGES, "root-2", {
      spaceId: SPACE,
      parentId: undefined,
      index: 0,
    });
    expect(ids(siblings(moved, SPACE, TOP_LEVEL))).toEqual([
      "root-2",
      "root-1",
    ]);
    expect(byId(moved, "root-2")?.position).toBe(1);
    expect(byId(moved, "root-1")?.position).toBe(2);
  });

  it("re-parents a page and clamps the index", () => {
    const moved = movePage(PAGES, "root-2", {
      spaceId: SPACE,
      parentId: "root-1",
      index: 99,
    });
    expect(ids(siblings(moved, SPACE, "root-1"))).toEqual([
      "child-1",
      "child-2",
      "root-2",
    ]);
    expect(byId(moved, "root-2")?.parentId).toBe("root-1");
  });

  it("moves the whole subtree to another space", () => {
    const moved = movePage(PAGES, "child-1", {
      spaceId: OTHER_SPACE,
      parentId: "elsewhere",
      index: -5,
    });
    expect(byId(moved, "child-1")).toMatchObject({
      spaceId: OTHER_SPACE,
      parentId: "elsewhere",
      position: 1,
    });
    expect(byId(moved, "grandchild")?.spaceId).toBe(OTHER_SPACE);
    expect(ids(siblings(moved, SPACE, "root-1"))).toEqual(["child-2"]);
  });

  it("refuses to move a page into its own subtree", () => {
    const moved = movePage(PAGES, "root-1", {
      spaceId: SPACE,
      parentId: "grandchild",
      index: 0,
    });
    expect(moved).toEqual(PAGES);
    expect(moved).not.toBe(PAGES);
  });

  it(
    "refuses an unknown page, a missing parent or a parent in another " +
      "space",
    () => {
      const target = { spaceId: SPACE, parentId: undefined, index: 0 };
      expect(movePage(PAGES, "unknown", target)).toEqual(PAGES);
      expect(
        movePage(PAGES, "root-2", { ...target, parentId: "missing" }),
      ).toEqual(PAGES);
      expect(
        movePage(PAGES, "root-2", { ...target, parentId: "elsewhere" }),
      ).toEqual(PAGES);
    },
  );
});

describe("targetFor", () => {
  it("moves up and down among siblings", () => {
    expect(targetFor(PAGES, "child-2", "up")).toEqual({
      spaceId: SPACE,
      parentId: "root-1",
      index: 0,
    });
    expect(targetFor(PAGES, "child-1", "down")).toEqual({
      spaceId: SPACE,
      parentId: "root-1",
      index: 1,
    });
  });

  it("cannot move the first page up or the last page down", () => {
    expect(targetFor(PAGES, "child-1", "up")).toBeUndefined();
    expect(targetFor(PAGES, "child-2", "down")).toBeUndefined();
  });

  it("indents under the previous sibling, at the end of its children", () => {
    expect(targetFor(PAGES, "child-2", "indent")).toEqual({
      spaceId: SPACE,
      parentId: "child-1",
      index: 1,
    });
    expect(targetFor(PAGES, "child-1", "indent")).toBeUndefined();
  });

  it("outdents to just after the parent", () => {
    expect(targetFor(PAGES, "grandchild", "outdent")).toEqual({
      spaceId: SPACE,
      parentId: "root-1",
      index: 1,
    });
    expect(targetFor(PAGES, "root-1", "outdent")).toBeUndefined();
  });

  it("returns undefined for an unknown page", () => {
    expect(targetFor(PAGES, "unknown", "up")).toBeUndefined();
  });
});
