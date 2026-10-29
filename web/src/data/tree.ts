// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { MoveTarget, PageMeta } from "./types";

/** A page with its children, for the sidebar. */
export interface TreeNode {
  readonly page: PageMeta;
  readonly children: readonly TreeNode[];
}

const byPosition = (a: PageMeta, b: PageMeta) =>
  a.position - b.position || a.title.localeCompare(b.title);

/** The pages under one parent, in order. */
export function siblings(
  pages: readonly PageMeta[],
  spaceId: string,
  parentId: string | undefined,
) {
  return pages.filter((p) => p.spaceId === spaceId && p.parentId === parentId).toSorted(byPosition);
}

/** Builds the tree of one space. Orphans (a missing parent) are shown at the top level. */
export function buildTree(pages: readonly PageMeta[], spaceId: string): TreeNode[] {
  const inSpace = pages.filter((p) => p.spaceId === spaceId);
  const ids = new Set(inSpace.map((p) => p.id));
  const children = new Map<string | undefined, PageMeta[]>();
  for (const page of inSpace) {
    const key = page.parentId !== undefined && ids.has(page.parentId) ? page.parentId : undefined;
    children.set(key, [...(children.get(key) ?? []), page]);
  }
  const build = (parent: string | undefined, seen: ReadonlySet<string>): TreeNode[] =>
    (children.get(parent) ?? [])
      .toSorted(byPosition)
      .filter((page) => !seen.has(page.id))
      .map((page) => ({ page, children: build(page.id, new Set([...seen, page.id])) }));
  return build(undefined, new Set());
}

/** The ids of a page and everything under it. */
export function descendants(pages: readonly PageMeta[], id: string): Set<string> {
  const out = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const page of pages) {
      if (page.parentId !== undefined && out.has(page.parentId) && !out.has(page.id)) {
        out.add(page.id);
        grew = true;
      }
    }
  }
  return out;
}

/** The chain of ancestors of a page, root first. */
export function ancestors(pages: readonly PageMeta[], id: string): PageMeta[] {
  const byId = new Map(pages.map((p) => [p.id, p]));
  const chain: PageMeta[] = [];
  let current = byId.get(id);
  const seen = new Set<string>();
  while (current?.parentId != undefined && !seen.has(current.parentId)) {
    seen.add(current.parentId);
    current = byId.get(current.parentId);
    if (current) {
      chain.unshift(current);
    }
  }
  return chain;
}

/**
 * Moves a page (and its subtree) to a new parent and index. Returns the
 * pages unchanged when the move is impossible, such as into its own subtree.
 */
export function movePage(pages: readonly PageMeta[], id: string, target: MoveTarget): PageMeta[] {
  const page = pages.find((p) => p.id === id);
  if (!page || (target.parentId !== undefined && descendants(pages, id).has(target.parentId))) {
    return [...pages];
  }
  const parent =
    target.parentId === undefined ? undefined : pages.find((p) => p.id === target.parentId);
  if (target.parentId !== undefined && parent?.spaceId !== target.spaceId) {
    return [...pages];
  }
  const others = siblings(pages, target.spaceId, target.parentId).filter((p) => p.id !== id);
  const index = Math.max(0, Math.min(target.index, others.length));
  const order = [...others.slice(0, index), page, ...others.slice(index)];
  const positions = new Map(order.map((p, index_) => [p.id, index_ + 1]));
  const moved = descendants(pages, id);
  return pages.map((p) => {
    const position = positions.get(p.id);
    if (p.id === id) {
      return { ...p, spaceId: target.spaceId, parentId: target.parentId, position: position ?? 1 };
    }
    if (moved.has(p.id)) {
      return { ...p, spaceId: target.spaceId };
    }
    return position === undefined ? p : { ...p, position };
  });
}

/** The keyboard and menu moves of the tree (a single-pointer alternative to dragging). */
export type TreeMove = "up" | "down" | "indent" | "outdent";

/** Where a page goes for a keyboard move, or `undefined` when it cannot move that way. */
export function targetFor(
  pages: readonly PageMeta[],
  id: string,
  move: TreeMove,
): MoveTarget | undefined {
  const page = pages.find((p) => p.id === id);
  if (!page) {
    return undefined;
  }
  const row = siblings(pages, page.spaceId, page.parentId);
  const index = row.findIndex((p) => p.id === id);
  const base = { spaceId: page.spaceId, parentId: page.parentId };
  switch (move) {
    case "up": {
      return index > 0 ? { ...base, index: index - 1 } : undefined;
    }
    case "down": {
      return index < row.length - 1 ? { ...base, index: index + 1 } : undefined;
    }
    case "indent": {
      const previous = row[index - 1];
      if (!previous) {
        return undefined;
      }
      const count = siblings(pages, page.spaceId, previous.id).length;
      return { spaceId: page.spaceId, parentId: previous.id, index: count };
    }
    case "outdent": {
      const parent = pages.find((p) => p.id === page.parentId);
      if (!parent) {
        return undefined;
      }
      const parentRow = siblings(pages, parent.spaceId, parent.parentId);
      const at = parentRow.findIndex((p) => p.id === parent.id);
      return { spaceId: parent.spaceId, parentId: parent.parentId, index: at + 1 };
    }
  }
}
