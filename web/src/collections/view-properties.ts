// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { CollectionInfo, PropertyType, RowView, ViewResult } from "./types";

/** What every view layout gets. */
export interface ViewProperties {
  readonly info: CollectionInfo;
  readonly result: ViewResult;
  /** Opens a row in the side panel. */
  readonly onOpen: (rowId: string) => void;
}

/** Every row of a result once, in order (a row can sit in several groups). */
export function uniqueRows(result: ViewResult): RowView[] {
  const seen = new Set<string>();
  return result.groups
    .flatMap((group) => group.rows)
    .filter((row) => !seen.has(row.id) && Boolean(seen.add(row.id)));
}

/** The material symbol for each property type. */
export const TYPE_ICONS: Readonly<Record<PropertyType, string>> = {
  title: "description",
  text: "notes",
  number: "calculate",
  select: "expand_circle_down",
  multi: "checklist",
  person: "person",
  date: "calendar_today",
  checkbox: "check_box",
  url: "link",
  relation: "polyline",
  rollup: "function",
  formula: "function",
  created: "history",
};
