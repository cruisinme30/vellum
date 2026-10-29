// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** The book's outline as rows, and where a drag or a key press moves a section. */

import type { Place, TocLine, TocNode } from "./api";

/** A section in the outline, with where it sits. */
export interface OutlineRow {
  readonly node: TocNode;
  readonly id: string;
  /** 1 at the top. */
  readonly level: number;
  readonly parent: string | undefined;
  /** Sections with the same parent (or the same top-level run), in order. */
  readonly siblings: readonly string[];
  readonly index: number;
  /** How many levels the section and everything under it span, 1 for a leaf. */
  readonly height: number;
}

export const isNode = (line: TocLine): line is TocNode => !("kind" in line);

/** A section that can be opened and moved: it has an id and is not a draft. */
const movable = (node: TocNode): node is TocNode & { id: string } =>
  node.id != undefined && !node.draft;

const heightOf = (node: TocNode): number =>
  1 + Math.max(0, ...node.children.filter(isNode).map((child) => heightOf(child)));

/** Every section in reading order. Parts and separators split top-level runs. */
export function outlineRows(lines: readonly TocLine[]): OutlineRow[] {
  const rows: OutlineRow[] = [];
  const walk = (nodes: readonly TocNode[], level: number, parent?: string) => {
    const ids = nodes.filter((n) => movable(n)).map((n) => n.id);
    for (const node of nodes) {
      if (!movable(node)) continue;
      rows.push({
        node,
        id: node.id,
        level,
        parent,
        siblings: ids,
        index: ids.indexOf(node.id),
        height: heightOf(node),
      });
      walk(node.children.filter(isNode), level + 1, node.id);
    }
  };
  let run: TocNode[] = [];
  for (const line of lines) {
    if (isNode(line)) {
      run.push(line);
    } else {
      walk(run, 1);
      run = [];
    }
  }
  walk(run, 1);
  return rows;
}

/** A keyboard move: up and down among siblings, in under the one above, or out. */
export type OutlineMove = "up" | "down" | "indent" | "outdent";

/**
 * Where `move` takes section `id`, or `undefined` when it cannot go there
 * (the first sibling going up, a section already at the top going out, or
 * nesting past `maxDepth`).
 */
export function placeFor(
  rows: readonly OutlineRow[],
  id: string,
  move: OutlineMove,
  maxDepth: number,
): Place | undefined {
  const row = rows.find((r) => r.id === id);
  if (row === undefined) return undefined;
  const before = row.siblings[row.index - 1];
  const after = row.siblings[row.index + 1];
  switch (move) {
    case "up": {
      return before === undefined ? undefined : { at: "before", section: before };
    }
    case "down": {
      return after === undefined ? undefined : { at: "after", section: after };
    }
    case "indent": {
      if (before === undefined || row.level + row.height > maxDepth) return undefined;
      return { at: "last_child_of", section: before };
    }
    case "outdent": {
      return row.parent === undefined ? undefined : { at: "after", section: row.parent };
    }
  }
}

/** Where a drop on `target` lands: its top quarter is before, its bottom quarter after, else inside. */
export function dropPlace(
  target: OutlineRow,
  dragged: OutlineRow,
  offset: number,
  height: number,
  maxDepth: number,
): Place | undefined {
  if (target.id === dragged.id) return undefined;
  if (offset < height * 0.25) return { at: "before", section: target.id };
  if (offset > height * 0.75) return { at: "after", section: target.id };
  if (target.level + dragged.height > maxDepth) return undefined;
  return { at: "last_child_of", section: target.id };
}

/** Whether `id` is `ancestor` or sits somewhere under it. */
export function isWithin(rows: readonly OutlineRow[], id: string, ancestor: string): boolean {
  let current: string | undefined = id;
  while (current !== undefined) {
    if (current === ancestor) return true;
    const parent: string | undefined = rows.find((r) => r.id === current)?.parent;
    current = parent;
  }
  return false;
}

/** Splits `user:…, group:…` text into principals, dropping blanks. */
export function principalList(text: string): string[] {
  return text
    .split(/[\s,]+/u)
    .map((p) => p.trim())
    .filter((p) => p !== "");
}
