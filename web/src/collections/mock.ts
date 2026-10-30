// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { newPageId } from "../data/store";

import {
  northstarCollections,
  type MockCollection,
  type MockRow,
  type MockSeed,
} from "./mock-data";
import {
  addProperty,
  addRelation,
  collectionFromCsv,
  csvOf,
  csvRows,
  deleteProperty,
  deleteView,
  embedQuery,
  saveView,
  updateProperty,
} from "./mock-edits";
import { computeRow, resolve, type MockWorld } from "./mock-engine";
import { runQuery } from "./mock-query";
import type { CollectionsStore } from "./store";
import type { CollectionInfo, CsvImported, PropertyInfo, RowDetail, Value } from "./types";
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
  let collections: readonly MockCollection[] = seed.collections;
  const world: MockWorld = {
    get collections() {
      return collections;
    },
    rows,
  };
  const listeners = new Set<() => void>();

  const collectionOf = (id: string) => {
    const collection = collections.find((c) => c.id === id);
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
  /** Swaps in new versions of these collections. */
  const replaceCollections = (...next: MockCollection[]) => {
    collections = collections.map((c) => next.find((n) => n.id === c.id) ?? c);
  };
  /** Replaces a collection's schema, as a `schema.yaml` rewrite would. */
  const edit = (id: string, change: (c: MockCollection) => MockCollection) =>
    later(() => {
      const next = change(collectionOf(id));
      collections = collections.map((c) => (c.id === id ? next : c));
      changed();
      return strip(next);
    });
  /** Adds parsed CSV rows to a collection. */
  const addRows = (collection: MockCollection, csv: string, write: boolean): CsvImported => {
    const parsed = csvRows(collection, csv);
    const added = parsed.rows.map((values): MockRow => ({
      id: newPageId(),
      values: { name: text(""), ...values },
      body: "",
    }));
    if (write) {
      rows.set(collection.id, [...(rows.get(collection.id) ?? []), ...added]);
      changed();
    }
    return {
      written: write,
      rows: added.length,
      skippedColumns: parsed.skipped,
      diagnostics: [],
      collection: strip(collection),
      changed: {
        written: write ? added.map((r) => ({ collection: collection.id, row: r.id })) : [],
        deleted: [],
      },
    };
  };

  return {
    list: () =>
      later(() => ({
        collections: collections
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
    deleteRow: (key) =>
      later(() => {
        const collection = collectionOf(key.collection_id);
        const row = rowOf(collection.id, key.row_id);
        for (const property of collection.properties) {
          mirror(property, row.id, row.values[property.key], EMPTY);
        }
        rows.set(
          collection.id,
          (rows.get(collection.id) ?? []).filter((r) => r.id !== row.id),
        );
        changed();
        return {
          changed: { written: [], deleted: [{ collection: collection.id, row: row.id }] },
        };
      }),
    saveView: (input) => edit(input.collection_id, (c) => saveView(c, input)),
    deleteView: (key) => edit(key.collection_id, (c) => deleteView(c, key.view)),
    addProperty: (input) => {
      if (input.type !== "relation") {
        return edit(input.collection_id, (c) => addProperty(c, input));
      }
      return later(() => {
        const [source, target] = addRelation(
          collectionOf(input.collection_id),
          collectionOf(input.to ?? ""),
          input,
        );
        replaceCollections(source, target);
        changed();
        return strip(source);
      });
    },
    updateProperty: (input) => edit(input.collection_id, (c) => updateProperty(c, input)),
    deleteProperty: (key) =>
      later(() => {
        const source = collectionOf(key.collection_id);
        const relation = source.properties.find((p) => p.key === key.key)?.relation;
        const next = deleteProperty(source, key.key);
        const back = relation?.back ?? "";
        if (relation && back !== "" && relation.to === source.id) {
          replaceCollections(deleteProperty(next, back));
        } else if (relation && back !== "") {
          replaceCollections(next, deleteProperty(collectionOf(relation.to), back));
        } else {
          replaceCollections(next);
        }
        changed();
        return strip(collectionOf(source.id));
      }),
    exportCsv: (input) =>
      later(() => {
        const collection = collectionOf(input.collection_id);
        const result = runQuery(world, collection, {
          ...input,
          limit: Number.MAX_SAFE_INTEGER,
        });
        const stem =
          input.view === undefined ? collection.title : `${collection.title} - ${result.view}`;
        return { fileName: `${stem}.csv`, csv: csvOf(result) };
      }),
    importCsv: (input) =>
      later(() => addRows(collectionOf(input.collection_id), input.csv, input.dry_run !== true)),
    createFromCsv: (input) =>
      later(() => {
        const title = input.title.trim();
        if (title === "") {
          throw new Error("a collection needs a title");
        }
        if (collections.some((c) => c.title.toLowerCase() === title.toLowerCase())) {
          throw new Error(`a collection named ${JSON.stringify(title)} is already there`);
        }
        const collection = collectionFromCsv(newPageId(), title, input.folder, input.csv);
        const write = input.dry_run !== true;
        if (write) {
          collections = [...collections, collection];
          rows.set(collection.id, []);
        }
        return addRows(collection, input.csv, write);
      }),
    embed: (fence) =>
      later(() => {
        const query = embedQuery(fence);
        const collection = collectionOf(query.collection_id);
        return {
          collectionId: collection.id,
          title: collection.title,
          path: collection.path,
          result: runQuery(world, collection, query),
        };
      }),
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
