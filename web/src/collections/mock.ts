// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { newPageId } from "../data/store";

import {
  northstarCollections,
  type MockCollection,
  type MockRow,
  type MockSeed,
} from "./mock-data";
import { computeRow, resolve, type MockWorld } from "./mock-engine";
import { runQuery } from "./mock-query";
import type { CollectionsStore } from "./store";
import type { CollectionInfo, PropertyInfo, RowDetail, Value } from "./types";
import { EMPTY, isEmpty, list, plain, text, texts } from "./value";

/** Checks that a value fits a property, as the server does on a write. */
function check(world: MockWorld, property: PropertyInfo, value: Value): void {
  if (property.computed) {
    throw new Error(`${property.name} is computed and cannot be set`);
  }
  if (isEmpty(value)) {
    return;
  }
  const want: Partial<Record<PropertyInfo["type"], Value["type"]>> = {
    number: "number",
    checkbox: "bool",
    date: "date",
    multi: "list",
    relation: "list",
  };
  if (value.type !== (want[property.type] ?? "text")) {
    throw new Error(`${property.name} expects a ${property.type} value, not ${value.type}`);
  }
  const names = property.options?.map((o) => o.name);
  const wrong = names ? texts(value).find((item) => !names.includes(item)) : undefined;
  if (wrong !== undefined) {
    throw new Error(`"${wrong}" is not an option of ${property.name}`);
  }
  const targets = world.rows.get(property.relation?.to ?? "");
  const missing = targets
    ? texts(value).find((id) => !targets.some((r) => r.id === id))
    : undefined;
  if (missing !== undefined) {
    throw new Error(`${property.name} links to a row that does not exist: ${missing}`);
  }
}

const strip = (collection: MockCollection): CollectionInfo => ({
  id: collection.id,
  title: collection.title,
  path: collection.path,
  properties: collection.properties,
  views: collection.views,
  diagnostics: [],
});

/** Answers soon but not at once, like the network would. */
const later = <T>(make: () => T): Promise<T> => Promise.resolve().then(make);

/**
 * An in-memory collections store with the Northstar demo: the mock adapter
 * of the collections port. Two-way relations are mirrored on every write.
 */
export function createMockCollectionsStore(
  seed: MockSeed = northstarCollections(),
): CollectionsStore {
  const rows = new Map<string, MockRow[]>(
    seed.collections.map((c) => [c.id, [...(seed.rows[c.id] ?? [])]]),
  );
  const world: MockWorld = { collections: seed.collections, rows };
  const listeners = new Set<() => void>();

  const collectionOf = (id: string) => {
    const collection = seed.collections.find((c) => c.id === id);
    if (!collection) {
      throw new Error(`collection ${id} not found`);
    }
    return collection;
  };
  const rowOf = (collectionId: string, rowId: string) => {
    const row = rows.get(collectionId)?.find((r) => r.id === rowId);
    if (!row) {
      throw new Error(`row ${rowId} not found`);
    }
    return row;
  };
  const replace = (collectionId: string, row: MockRow) => {
    rows.set(
      collectionId,
      (rows.get(collectionId) ?? []).map((r) => (r.id === row.id ? row : r)),
    );
  };
  /** Adds or removes `rowId` in the paired property of the rows it links to. */
  const mirror = (
    property: PropertyInfo,
    rowId: string,
    before: Value | undefined,
    after: Value,
  ) => {
    const { to, back } = property.relation ?? {};
    if (to === undefined || back === undefined) {
      return;
    }
    const [was, now] = [new Set(texts(before)), new Set(texts(after))];
    for (const target of rows.get(to) ?? []) {
      const linked = new Set(texts(target.values[back]));
      const wanted = now.has(target.id) || (linked.has(rowId) && !was.has(target.id));
      if (wanted !== linked.has(rowId)) {
        const next = wanted ? [...linked, rowId] : [...linked].filter((id) => id !== rowId);
        replace(to, {
          ...target,
          values: { ...target.values, [back]: list(next.map((id) => text(id))) },
        });
      }
    }
  };
  const write = (
    collection: MockCollection,
    row: MockRow,
    values: Readonly<Record<string, Value>>,
  ) => {
    const next: Record<string, Value> = { ...row.values };
    for (const [name, value] of Object.entries(values)) {
      const property = resolve(collection, name);
      if (!property) {
        throw new Error(`no property ${JSON.stringify(name)}`);
      }
      check(world, property, value);
      mirror(property, row.id, row.values[property.key], value);
      next[property.key] = value;
    }
    return { ...row, values: next };
  };
  const detail = (collection: MockCollection, row: MockRow): RowDetail => {
    const full = computeRow(world, collection, row);
    return {
      collectionId: collection.id,
      path: `${collection.path}/rows/${row.id}.md`,
      title: plain(full.values["name"]),
      row: {
        id: row.id,
        cells: Object.fromEntries(
          collection.properties.map((p) => [p.key, { value: full.values[p.key] ?? EMPTY }]),
        ),
      },
      body: row.body,
    };
  };
  const changed = () => {
    for (const listener of listeners) {
      listener();
    }
  };

  return {
    list: () =>
      later(() => ({
        collections: seed.collections
          .map((c) => ({
            id: c.id,
            title: c.title,
            path: c.path,
            rows: rows.get(c.id)?.length ?? 0,
            views: c.views.map((v) => v.name),
          }))
          .toSorted((a, b) => a.title.localeCompare(b.title)),
      })),
    info: (id) => later(() => strip(collectionOf(id))),
    query: (input) => later(() => runQuery(world, collectionOf(input.collection_id), input)),
    getRow: (key) =>
      later(() => detail(collectionOf(key.collection_id), rowOf(key.collection_id, key.row_id))),
    createRow: (input) =>
      later(() => {
        const collection = collectionOf(input.collection_id);
        const blank: MockRow = {
          id: newPageId(),
          values: { name: text("") },
          body: input.body ?? "",
        };
        const row = write(collection, blank, input.values ?? {});
        rows.set(collection.id, [...(rows.get(collection.id) ?? []), row]);
        changed();
        return {
          row: detail(collection, row),
          changed: { written: [{ collection: collection.id, row: row.id }], deleted: [] },
        };
      }),
    updateRow: (input) =>
      later(() => {
        const collection = collectionOf(input.collection_id);
        const row = write(collection, rowOf(collection.id, input.row_id), input.values ?? {});
        replace(collection.id, { ...row, body: input.body ?? row.body });
        changed();
        return {
          row: detail(collection, rowOf(collection.id, row.id)),
          changed: { written: [{ collection: collection.id, row: row.id }], deleted: [] },
        };
      }),
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
