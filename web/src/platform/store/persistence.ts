// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { type DBSchema, type IDBPDatabase, openDB } from "idb";

import type { JsonValue, Patch, PendingMutation } from "./types";

/** One stored Yjs update for a document. */
interface DocumentUpdateRow {
  readonly documentId: string;
  readonly update: Uint8Array;
}

/** Bookkeeping for one cached document. */
export interface DocumentEntry {
  readonly documentId: string;
  readonly spaceId: string;
  readonly bytes: number;
  readonly lastAccess: number;
}

interface LocalSchema extends DBSchema {
  records: {
    key: [string, string];
    value: { collection: string; key: string; value: JsonValue };
  };
  outbox: { key: number; value: PendingMutation };
  meta: { key: string; value: string };
  documentUpdates: {
    key: number;
    value: DocumentUpdateRow;
    indexes: { byDoc: string };
  };
  documents: {
    key: string;
    value: DocumentEntry;
    indexes: { byAccess: number; bySpace: string };
  };
  pins: { key: string; value: { spaceId: string } };
}

/** The IndexedDB database behind one signed-in account on one server. */
export type LocalDatabase = IDBPDatabase<LocalSchema>;

const VERSION = 1;

/**
 * Opens (and creates or upgrades) the local database.
 *
 * @param name - one database per server origin and account, so signing in
 *   as someone else never shows the previous person's cache
 */
export async function openLocalDatabase(name: string): Promise<LocalDatabase> {
  return openDB<LocalSchema>(name, VERSION, {
    upgrade(database) {
      database.createObjectStore("records", { keyPath: ["collection", "key"] });
      database.createObjectStore("outbox", { keyPath: "id" });
      database.createObjectStore("meta");
      const updates = database.createObjectStore("documentUpdates", { autoIncrement: true });
      updates.createIndex("byDoc", "documentId");
      const documents = database.createObjectStore("documents", { keyPath: "documentId" });
      documents.createIndex("byAccess", "lastAccess");
      documents.createIndex("bySpace", "spaceId");
      database.createObjectStore("pins", { keyPath: "spaceId" });
    },
  });
}

/** Everything the store needs at start-up. */
export interface Snapshot {
  readonly patches: readonly Patch[];
  readonly pending: readonly PendingMutation[];
  readonly meta: ReadonlyMap<string, string>;
}

/** Reads the whole base state, the outbox and the metadata. */
export async function readSnapshot(database: LocalDatabase): Promise<Snapshot> {
  const tx = database.transaction(["records", "outbox", "meta"]);
  const [records, pending, metaKeys, metaValues] = await Promise.all([
    tx.objectStore("records").getAll(),
    tx.objectStore("outbox").getAll(),
    tx.objectStore("meta").getAllKeys(),
    tx.objectStore("meta").getAll(),
  ]);
  const meta = new Map<string, string>();
  for (const [index, key] of metaKeys.entries()) meta.set(key, metaValues[index] ?? "");
  const patches = records.map((row): Patch => ({
    op: "put",
    collection: row.collection,
    key: row.key,
    value: row.value,
  }));
  return { patches, pending, meta };
}

/** Appends a mutation to the outbox. */
export async function enqueue(database: LocalDatabase, mutation: PendingMutation): Promise<void> {
  await database.put("outbox", mutation);
}

/** Writes metadata (client ID, next mutation ID, cursor). */
export async function writeMeta(
  database: LocalDatabase,
  entries: Readonly<Record<string, string>>,
): Promise<void> {
  const tx = database.transaction("meta", "readwrite");
  await Promise.all([
    ...Object.entries(entries).map(([key, value]) => tx.store.put(value, key)),
    tx.done,
  ]);
}

/**
 * Commits a pull in one transaction: patches into the base, confirmed
 * mutations out of the outbox, and the new cursor.
 */
export async function commitPull(
  database: LocalDatabase,
  pull: { readonly patches: readonly Patch[]; readonly reset: boolean; readonly cursor: string },
  confirmedThrough: number,
): Promise<void> {
  const tx = database.transaction(["records", "outbox", "meta"], "readwrite");
  const records = tx.objectStore("records");
  const writes: Promise<unknown>[] = [];
  if (pull.reset) writes.push(records.clear());
  for (const patch of pull.patches) {
    writes.push(
      patch.op === "put"
        ? records.put({ collection: patch.collection, key: patch.key, value: patch.value })
        : records.delete([patch.collection, patch.key]),
    );
  }
  writes.push(
    tx.objectStore("outbox").delete(IDBKeyRange.upperBound(confirmedThrough)),
    tx.objectStore("meta").put(pull.cursor, "cursor"),
    tx.objectStore("meta").put(String(confirmedThrough), "confirmed"),
    tx.done,
  );
  await Promise.all(writes);
}

/** Removes mutations from the outbox, for example after a rejection. */
export async function dropMutations(
  database: LocalDatabase,
  ids: readonly number[],
): Promise<void> {
  const tx = database.transaction("outbox", "readwrite");
  await Promise.all([...ids.map((id) => tx.store.delete(id)), tx.done]);
}
