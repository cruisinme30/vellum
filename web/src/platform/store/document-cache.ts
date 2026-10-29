// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { telemetry } from "../telemetry";

import type { DocumentEntry, LocalDatabase } from "./persistence";

/** Options for {@link DocumentCache}. */
export interface DocumentCacheOptions {
  /**
   * Byte budget for unpinned documents. The web app keeps recent pages
   * within it; the desktop pins whole spaces, which are never evicted.
   */
  readonly budgetBytes?: number;
  /** Clock, for tests. */
  readonly now?: () => number;
}

const DEFAULT_BUDGET = 50 * 1024 * 1024;

/**
 * Offline copy of documents as Yjs updates, opaque bytes to this module.
 *
 * The editor's collaboration provider appends every update it sees and
 * loads them back on open, so a page opens instantly and offline edits
 * survive a reload; the CRDT merges them with the server on reconnect.
 */
export class DocumentCache {
  readonly #database: LocalDatabase;
  readonly #budget: number;
  readonly #now: () => number;

  /** @param database - the account's local database */
  constructor(database: LocalDatabase, options: DocumentCacheOptions = {}) {
    this.#database = database;
    this.#budget = options.budgetBytes ?? DEFAULT_BUDGET;
    this.#now = options.now ?? Date.now;
  }

  /** Every stored update of a document, oldest first. Empty when not cached. */
  async load(documentId: string): Promise<Uint8Array[]> {
    const tx = this.#database.transaction(["documentUpdates", "documents"], "readwrite");
    const rows = await tx.objectStore("documentUpdates").index("byDoc").getAll(documentId);
    const entry = await tx.objectStore("documents").get(documentId);
    if (entry !== undefined)
      await tx.objectStore("documents").put({ ...entry, lastAccess: this.#now() });
    await tx.done;
    telemetry().count("vellum.client.doc_cache.load", { hit: rows.length > 0 });
    return rows.map((row) => row.update);
  }

  /** Stores one more update of a document. */
  async append(documentId: string, update: Uint8Array): Promise<void> {
    const tx = this.#database.transaction(["documentUpdates", "documents"], "readwrite");
    const documents = tx.objectStore("documents");
    const entry = await documents.get(documentId);
    await Promise.all([
      tx.objectStore("documentUpdates").add({ documentId, update }),
      documents.put({
        documentId,
        spaceId: entry?.spaceId ?? "",
        bytes: (entry?.bytes ?? 0) + update.byteLength,
        lastAccess: this.#now(),
      }),
      tx.done,
    ]);
    await this.evict();
  }

  /** Replaces every stored update of a document with one merged state. */
  async compact(documentId: string, merged: Uint8Array): Promise<void> {
    const tx = this.#database.transaction(["documentUpdates", "documents"], "readwrite");
    const updates = tx.objectStore("documentUpdates");
    const keys = await updates.index("byDoc").getAllKeys(documentId);
    const entry = await tx.objectStore("documents").get(documentId);
    await Promise.all([
      ...keys.map((key) => updates.delete(key)),
      updates.add({ documentId, update: merged }),
      tx.objectStore("documents").put({
        documentId,
        spaceId: entry?.spaceId ?? "",
        bytes: merged.byteLength,
        lastAccess: this.#now(),
      }),
      tx.done,
    ]);
  }

  /** Records which space a document belongs to, so pinning covers it. */
  async track(documentId: string, spaceId: string): Promise<void> {
    const documents = this.#database.transaction("documents", "readwrite").store;
    const entry = await documents.get(documentId);
    await documents.put({
      documentId,
      spaceId,
      bytes: entry?.bytes ?? 0,
      lastAccess: entry?.lastAccess ?? this.#now(),
    });
  }

  /** Keeps every document of a space offline, whatever the budget. */
  async pin(spaceId: string): Promise<void> {
    await this.#database.put("pins", { spaceId });
  }

  /** Lets a space's documents be evicted again. */
  async unpin(spaceId: string): Promise<void> {
    await this.#database.delete("pins", spaceId);
    await this.evict();
  }

  /** Spaces kept offline. */
  async pinned(): Promise<string[]> {
    const pins = await this.#database.getAll("pins");
    return pins.map((pin) => pin.spaceId);
  }

  /** Removes a document, for example after it is deleted or access is lost. */
  async remove(documentId: string): Promise<void> {
    const tx = this.#database.transaction(["documentUpdates", "documents"], "readwrite");
    const updates = tx.objectStore("documentUpdates");
    const keys = await updates.index("byDoc").getAllKeys(documentId);
    await Promise.all([
      ...keys.map((key) => updates.delete(key)),
      tx.objectStore("documents").delete(documentId),
      tx.done,
    ]);
  }

  /**
   * Drops the least recently used unpinned documents until the rest fit
   * the budget.
   *
   * @returns the IDs of the evicted documents
   */
  async evict(): Promise<string[]> {
    const pinnedSpaces = await this.pinned();
    const pins = new Set(pinnedSpaces);
    const entries: DocumentEntry[] = await this.#database.getAllFromIndex("documents", "byAccess");
    const unpinned = entries.filter((entry) => !pins.has(entry.spaceId));
    let total = unpinned.reduce((sum, entry) => sum + entry.bytes, 0);
    const evicted: string[] = [];
    for (const entry of unpinned) {
      if (total <= this.#budget) break;
      await this.remove(entry.documentId);
      total -= entry.bytes;
      evicted.push(entry.documentId);
    }
    if (evicted.length > 0)
      telemetry().count("vellum.client.doc_cache.evicted", { documents: evicted.length });
    return evicted;
  }
}
