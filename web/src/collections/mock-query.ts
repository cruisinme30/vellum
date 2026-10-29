// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { MockCollection } from "./mock-data";
import {
  columnsOf,
  computeRow,
  display,
  groupRows,
  resolve,
  sortRows,
  type FullRow,
  type Groupable,
  type MockWorld,
} from "./mock-engine";
import { matches, parseFilter } from "./mock-filter";
import type { CellView, Column, QueryInput, ViewInfo, ViewResult } from "./types";
import { dayOf, isEmpty, plain } from "./value";

function viewOf(collection: MockCollection, name: string | undefined): ViewInfo {
  const wanted = name?.trim().toLowerCase();
  const view =
    wanted === undefined
      ? collection.views[0]
      : collection.views.find((v) => v.name.toLowerCase() === wanted);
  if (!view) {
    throw new Error(`no view named ${JSON.stringify(name ?? "")}`);
  }
  return view;
}

/** The rows a view keeps, in its order. */
function selectRows(
  world: MockWorld,
  collection: MockCollection,
  view: ViewInfo,
  input: QueryInput,
): FullRow[] {
  const filters = [...view.filters, ...(input.filter ?? [])].map((f) => parseFilter(collection, f));
  const kept = (world.rows.get(collection.id) ?? [])
    .map((row) => computeRow(world, collection, row))
    .filter((row) =>
      filters.every((f) => {
        const property = resolve(collection, f.key);
        return property !== undefined && matches(world, property, row.values[f.key], f);
      }),
    );
  return sortRows(collection, kept, input.sort?.length ? input.sort : view.sorts);
}

function cellsOf(
  world: MockWorld,
  collection: MockCollection,
  columns: readonly Column[],
  row: FullRow,
): CellView[] {
  return columns.map((column) => {
    const value = row.values[column.key];
    const property = resolve(collection, column.key);
    const shown = property ? display(world, property, value) : "";
    return value === undefined || isEmpty(value) ? { display: shown } : { value, display: shown };
  });
}

/** Where a row sits on a calendar or timeline: from its date to its end date. */
function spanOf(row: FullRow, dateKey: string, endKey: string | undefined) {
  const start = dayOf(row.values[dateKey]);
  const end = dayOf(row.values[endKey ?? ""]);
  return start === "" ? {} : { span: { start, end: end.localeCompare(start) > 0 ? end : start } };
}

/** Runs a query the way `collections.query` does, on the mock's data. */
export function runQuery(
  world: MockWorld,
  collection: MockCollection,
  input: QueryInput,
): ViewResult {
  const view = viewOf(collection, input.view);
  const kind = input.kind ?? view.kind;
  const sorted = selectRows(world, collection, view, input);
  const limit = input.limit ?? view.limit ?? sorted.length;
  const columns = columnsOf(collection, input.hide?.length ? input.hide : view.hidden);
  const dateKey = view.date ?? collection.properties.find((p) => p.type === "date")?.key ?? "";
  const timed = kind === "calendar" || kind === "timeline";
  const group = resolve(collection, input.group ?? view.group ?? "");
  const rows = sorted.slice(0, limit).map((row): Groupable => ({
    id: row.id,
    title: plain(row.values["name"]),
    cells: cellsOf(world, collection, columns, row),
    ...(timed ? spanOf(row, dateKey, view.end) : {}),
    group: group ? row.values[group.key] : undefined,
  }));
  return {
    collection: collection.id,
    title: collection.title,
    view: view.name,
    kind,
    columns,
    groups: groupRows(world, group, rows, kind === "board"),
    total: sorted.length,
    truncated: sorted.length > limit,
  };
}
