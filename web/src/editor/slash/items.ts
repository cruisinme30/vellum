// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { NODES, SECTIONS } from "../schema/blocks.gen";
import type { SlashData } from "../schema/types";

/** One entry of the slash menu, generated from the schema. */
export interface SlashItem extends SlashData {
  /** Stable id: node name plus the entry's position. */
  readonly id: string;
  /** The node type it inserts. */
  readonly node: string;
}

/** Every slash entry, in section order then schema order. */
export const SLASH_ITEMS: readonly SlashItem[] = SECTIONS.flatMap((section) =>
  NODES.flatMap((node) =>
    node.slash
      .map((entry, index) => ({
        ...entry,
        id: `${node.name}-${String(index)}`,
        node: node.name,
      }))
      .filter((entry) => entry.section === section),
  ),
);

const normalise = (text: string) =>
  text
    .toLowerCase()
    .replaceAll(/[^a-z0-9]+/g, " ")
    .trim();

function score(item: SlashItem, query: string): number {
  const title = normalise(item.title);
  if (title === query) {
    return 0;
  }
  if (title.startsWith(query)) {
    return 1;
  }
  if (title.split(" ").some((word) => word.startsWith(query))) {
    return 2;
  }
  if (item.keywords.some((k) => normalise(k).startsWith(query))) {
    return 3;
  }
  if (title.includes(query) || normalise(item.description).includes(query)) {
    return 4;
  }
  return Number.POSITIVE_INFINITY;
}

/** Filters and ranks entries for what the user typed after `/`. */
export function filterSlashItems(
  raw: string,
  items: readonly SlashItem[] = SLASH_ITEMS,
) {
  const query = normalise(raw);
  if (query === "") {
    return [...items];
  }
  return items
    .map((item, index) => ({ item, index, rank: score(item, query) }))
    .filter((entry) => Number.isFinite(entry.rank))
    .toSorted((a, b) => a.rank - b.rank || a.index - b.index)
    .map((entry) => entry.item);
}
