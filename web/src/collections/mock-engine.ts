// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// A small stand-in for the server's view engine (view.rs): just enough
// filtering, sorting, grouping, hiding and limiting for the demo views.
import type { MockCollection, MockRow } from "./mock-data";
import type { Column, Group, PropertyInfo, RowView, Value } from "./types";
import {
  bool,
  EMPTY,
  formatNumber,
  isEmpty,
  number,
  plain,
  text,
  texts,
} from "./value";

/** All collections and their rows. */
export interface MockWorld {
  readonly collections: readonly MockCollection[];
  readonly rows: ReadonlyMap<string, readonly MockRow[]>;
}

/** A row with every value, stored and computed. */
export interface FullRow {
  readonly id: string;
  readonly values: Readonly<Record<string, Value>>;
}

/** Finds a property by key, or by display name ignoring case. */
export function resolve(
  collection: MockCollection,
  nameOrKey: string,
): PropertyInfo | undefined {
  const wanted = nameOrKey.trim();
  return (
    collection.properties.find((p) => p.key === wanted) ??
    collection.properties.find(
      (p) => p.name.toLowerCase() === wanted.toLowerCase(),
    )
  );
}

function rollup(
  world: MockWorld,
  property: PropertyInfo,
  values: Record<string, Value>,
): Value {
  const spec = property.rollup;
  const from = world.collections.find((c) =>
    c.properties.some((p) => p.key === spec?.via),
  );
  const via = from?.properties.find((p) => p.key === spec?.via);
  const targets = world.rows.get(via?.relation?.to ?? "") ?? [];
  const linked = new Set(texts(values[spec?.via ?? ""]));
  const items = targets
    .filter((row) => linked.has(row.id))
    .map((row) => row.values[spec?.prop ?? ""]);
  if (spec?.agg === "sum") {
    return number(
      items.reduce((sum, v) => sum + (v?.type === "number" ? v.value : 0), 0),
    );
  }
  return number(items.length);
}

/** A row's stored values plus its formulas and rollups. */
export function computeRow(
  world: MockWorld,
  collection: MockCollection,
  row: MockRow,
): FullRow {
  const values: Record<string, Value> = { ...row.values };
  for (const property of collection.properties) {
    const formula = collection.formulas[property.key];
    if (formula) {
      values[property.key] = formula(values);
    } else if (property.rollup) {
      values[property.key] = rollup(world, property, values);
    }
  }
  return { id: row.id, values };
}

/** The title of a row in any collection, by id. */
function titleOf(
  world: MockWorld,
  collectionId: string,
  rowId: string,
): string {
  const row = world.rows.get(collectionId)?.find((r) => r.id === rowId);
  return row ? plain(row.values["name"]) : rowId;
}

/**
 * The text items a value shows: related rows' titles, options, or the value.
 */
export function itemTexts(
  world: MockWorld,
  property: PropertyInfo,
  value: Value | undefined,
): string[] {
  if (property.relation) {
    const to = property.relation.to;
    return texts(value).map((id) => titleOf(world, to, id));
  }
  if (value?.type === "list") {
    return value.value
      .filter((item) => !isEmpty(item))
      .map((item) => plain(item));
  }
  return isEmpty(value) ? [] : [plain(value)];
}

/** How the server formats a cell for display. */
export function display(
  world: MockWorld,
  property: PropertyInfo,
  value: Value | undefined,
): string {
  if (value?.type === "number" && property.format === "percent") {
    return `${formatNumber(Math.round(value.value * 100))}%`;
  }
  return itemTexts(world, property, value).join(", ");
}

const RANK: Record<Value["type"], number> = {
  empty: 0,
  bool: 1,
  number: 2,
  date: 3,
  text: 4,
  list: 5,
};

/** The server's sort order: select options as listed, then by value. */
function compareValues(
  property: PropertyInfo | undefined,
  a: Value,
  b: Value,
): number {
  const names = (property?.options ?? []).map((o) => o.name);
  const rank = (value: Value) => {
    const at = names.indexOf(plain(value));
    return at === -1 ? names.length : at;
  };
  if (rank(a) < names.length || rank(b) < names.length) {
    return rank(a) - rank(b);
  }
  if (a.type === "number" && b.type === "number") {
    return a.value - b.value;
  }
  if (a.type !== b.type) {
    return RANK[a.type] - RANK[b.type];
  }
  return plain(a).toLowerCase().localeCompare(plain(b).toLowerCase());
}

/** Sorts rows by `key` or `key desc` sort texts; empty values go last. */
export function sortRows(
  collection: MockCollection,
  rows: readonly FullRow[],
  sorts: readonly string[],
): FullRow[] {
  const keys = sorts.map((source) => {
    const descending = / desc$/i.test(source);
    const property = resolve(collection, source.replace(/ (?:desc|asc)$/i, ""));
    return { property, descending };
  });
  return rows.toSorted((a, b) => {
    for (const { property, descending } of keys) {
      const [x, y] = [
        a.values[property?.key ?? ""] ?? EMPTY,
        b.values[property?.key ?? ""] ?? EMPTY,
      ];
      const sign = descending ? -1 : 1;
      const order =
        isEmpty(x) || isEmpty(y)
          ? Number(isEmpty(x)) - Number(isEmpty(y))
          : compareValues(property, x, y) * sign;
      if (order !== 0) {
        return order;
      }
    }
    return a.id.localeCompare(b.id);
  });
}

/**
 * Columns of a result: every property but the hidden ones (never the title).
 */
export function columnsOf(
  collection: MockCollection,
  hidden: readonly string[],
): Column[] {
  return collection.properties
    .filter((p) => p.type === "title" || !hidden.includes(p.key))
    .map((p) => ({
      key: p.key,
      name: p.name,
      type: p.type,
      format: p.format ?? "plain",
      ...(p.options ? { options: p.options } : {}),
      editable: !p.computed,
    }));
}

/** A row on its way into groups, with the value it is grouped by. */
export interface Groupable extends RowView {
  readonly group: Value | undefined;
}

/** The values a row is grouped by: one per item, none when empty. */
function groupValues(
  property: PropertyInfo,
  value: Value | undefined,
): readonly Value[] {
  if (property.type === "checkbox") {
    return [bool(value?.type === "bool" && value.value)];
  }
  if (property.relation) {
    return texts(value).map((item): Value => ({ type: "text", value: item }));
  }
  return value?.type === "list" ? value.value : [value ?? EMPTY];
}

/** The groups one value puts a row in, with their labels. */
function groupItems(
  world: MockWorld,
  property: PropertyInfo,
  value: Value | undefined,
) {
  return groupValues(property, value)
    .filter((item) => property.type === "checkbox" || !isEmpty(item))
    .map((item) => ({
      value: item,
      label:
        property.type === "checkbox"
          ? plain(item)
          : display(world, property, item),
    }));
}

/**
 * Groups rows by a property, as the server does: a board lists every
 * option first, other values follow in sort order, then the empty group.
 * Without a property, every row is in one unlabelled group.
 */
export function groupRows(
  world: MockWorld,
  property: PropertyInfo | undefined,
  rows: readonly Groupable[],
  board: boolean,
): Group[] {
  const keyed = rows.map(({ group, ...row }) => ({
    row,
    items: property ? groupItems(world, property, group) : [],
  }));
  if (!property) {
    // eslint-disable-next-line unicorn/no-null -- the server sends null for an ungrouped result
    return [{ value: null, label: "", rows: keyed.map((k) => k.row) }];
  }
  const options = board ? (property.options ?? []).map((o) => o.name) : [];
  const extra = keyed
    .flatMap((k) => k.items)
    .filter(
      (item, index, all) =>
        all.findIndex((other) => other.label === item.label) === index,
    )
    .filter((item) => !options.includes(item.label))
    .toSorted((a, b) => compareValues(property, a.value, b.value));
  const order = [
    ...options.map((name) => ({ value: text(name), label: name })),
    ...extra,
  ];
  const groups: Group[] = order.map(({ value, label }) => ({
    value,
    label,
    rows: keyed
      .filter((k) => k.items.some((item) => item.label === label))
      .map((k) => k.row),
  }));
  const none = keyed.filter((k) => k.items.length === 0).map((k) => k.row);
  if (none.length > 0 || (board && groups.length === 0)) {
    // eslint-disable-next-line unicorn/no-null -- the server sends null for the "No …" group
    groups.push({ value: null, label: `No ${property.name}`, rows: none });
  }
  return groups;
}
