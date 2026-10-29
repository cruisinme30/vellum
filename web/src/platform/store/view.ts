// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JsonValue, Mutators, Patch, PendingMutation, WriteTx } from "./types";

/** Marks a record deleted by a pending mutation. */
const TOMBSTONE = Symbol("tombstone");
type Slot = JsonValue | typeof TOMBSTONE;

type Table = Map<string, JsonValue>;

/** A slot as a reader sees it: deleted records read as missing. */
function reveal(slot: Slot | undefined): JsonValue | undefined {
  return slot === TOMBSTONE ? undefined : slot;
}

/** Orders record keys by UTF-16 code units, the same order on every device. */
function compareKeys(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

/** Error raised by `View.apply` when no mutator has the given name. */
export class UnknownMutatorError extends Error {
  /** @param mutatorName - the unregistered mutator name */
  constructor(readonly mutatorName: string) {
    super(`Vellum: no mutator named "${mutatorName}"`);
  }
}

/**
 * The in-memory state of the store: the confirmed server state ("base")
 * plus an overlay that holds the effect of every pending mutation.
 *
 * Reads go to the overlay first, so a mutation is visible the moment it
 * is applied, before anything reaches the disk or the network. When the
 * server confirms mutations, the overlay is rebuilt by replaying the ones
 * still pending on the new base, which is how concurrent edits from other
 * people are merged without losing local intent.
 */
export class View {
  readonly #base = new Map<string, Table>();
  #overlay = new Map<string, Map<string, Slot>>();

  /** @param mutators - the registered mutators, by name */
  constructor(private readonly mutators: Mutators) {}

  /** Reads a record, overlay first. */
  get(collection: string, key: string): JsonValue | undefined {
    const overlay = this.#overlay.get(collection);
    if (overlay?.has(key) === true) return reveal(overlay.get(key));
    return this.#base.get(collection)?.get(key);
  }

  /** Every live record of a collection, sorted by key. */
  scan(collection: string): readonly (readonly [string, JsonValue])[] {
    const merged = new Map<string, Slot>(this.#base.get(collection));
    for (const [key, slot] of this.#overlay.get(collection) ?? []) merged.set(key, slot);
    const rows: [string, JsonValue][] = [];
    for (const [key, slot] of merged) if (slot !== TOMBSTONE) rows.push([key, slot]);
    return rows.toSorted(([a], [b]) => compareKeys(a, b));
  }

  /**
   * Runs a mutator against the overlay.
   *
   * @returns the collections the mutation touched
   * @throws UnknownMutatorError when `name` is not registered; any error the
   *   mutator throws leaves the view unchanged
   */
  apply(mutation: Pick<PendingMutation, "name" | "args">): ReadonlySet<string> {
    const mutator = this.mutators[mutation.name];
    if (mutator === undefined) throw new UnknownMutatorError(mutation.name);
    const writes = new Map<string, Map<string, Slot>>();
    const tx = this.#tx(writes);
    mutator(tx, mutation.args);
    for (const [collection, rows] of writes) {
      const table = this.#overlay.get(collection) ?? new Map<string, Slot>();
      for (const [key, slot] of rows) table.set(key, slot);
      this.#overlay.set(collection, table);
    }
    return new Set(writes.keys());
  }

  /**
   * Applies server patches to the base and replays `pending` on top.
   * A mutation that now throws is skipped: the server is the authority and
   * will reject it too.
   *
   * @returns every collection whose visible state may have changed
   */
  rebase(
    patches: readonly Patch[],
    reset: boolean,
    pending: readonly PendingMutation[],
  ): Set<string> {
    const touched = new Set<string>(this.#overlay.keys());
    if (reset) {
      for (const collection of this.#base.keys()) touched.add(collection);
      this.#base.clear();
    }
    this.load(patches);
    for (const patch of patches) touched.add(patch.collection);
    this.#overlay = new Map();
    for (const mutation of pending) {
      try {
        for (const collection of this.apply(mutation)) touched.add(collection);
      } catch {
        // Replays are best effort; see the method comment.
      }
    }
    return touched;
  }

  /** Applies patches to the base without touching the overlay. */
  load(patches: readonly Patch[]): void {
    for (const patch of patches) {
      const table = this.#base.get(patch.collection) ?? new Map<string, JsonValue>();
      if (patch.op === "put") table.set(patch.key, patch.value);
      else table.delete(patch.key);
      this.#base.set(patch.collection, table);
    }
  }

  #tx(writes: Map<string, Map<string, Slot>>): WriteTx {
    const write = (collection: string, key: string, slot: Slot) => {
      const rows = writes.get(collection) ?? new Map<string, Slot>();
      rows.set(key, slot);
      writes.set(collection, rows);
    };
    const read = (collection: string, key: string) => {
      const own = writes.get(collection);
      if (own?.has(key) === true) return reveal(own.get(key));
      return this.get(collection, key);
    };
    return {
      get: read,
      scan: (collection) => {
        const own = writes.get(collection);
        if (own === undefined) return this.scan(collection);
        const keys = new Set([...this.scan(collection).map(([key]) => key), ...own.keys()]);
        const rows: [string, JsonValue][] = [];
        for (const key of [...keys].toSorted(compareKeys)) {
          const value = read(collection, key);
          if (value !== undefined) rows.push([key, value]);
        }
        return rows;
      },
      put: (collection, key, value) => {
        write(collection, key, value);
      },
      del: (collection, key) => {
        write(collection, key, TOMBSTONE);
      },
    };
  }
}
