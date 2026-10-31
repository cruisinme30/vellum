// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { telemetry } from "../telemetry";

import {
  commitPull,
  dropMutations,
  enqueue,
  type LocalDatabase,
  readSnapshot,
  writeMeta,
} from "./persistence";
import type { SyncTransport } from "./transport";
import type {
  JsonValue,
  Mutators,
  PendingMutation,
  ReadTx,
  RejectedMutation,
} from "./types";
import { View } from "./view";

/** Options for {@link LocalStore.open}. */
export interface LocalStoreOptions {
  readonly database: LocalDatabase;
  readonly mutators: Mutators;
  readonly transport: SyncTransport;
  /** Told about every mutation the server refused, to show the person. */
  readonly onRejected?: (
    rejected: RejectedMutation,
    mutation: PendingMutation | undefined,
  ) => void;
  /** Told when IndexedDB refuses a write (quota, private mode). */
  readonly onDiskError?: (error: unknown) => void;
  /** Clock, for tests. */
  readonly now?: () => number;
}

/** What one {@link LocalStore.sync} did. */
export interface SyncOutcome {
  readonly pushed: number;
  readonly pulled: number;
  readonly pending: number;
}

type Listener = () => void;

/**
 * The local-first store (D98). Every read is served from memory and every
 * write is applied to memory first, so the interface reacts in the same
 * frame. The disk (IndexedDB) and the server catch up afterwards: the
 * mutation is appended to an outbox, pushed with an idempotent ID, and
 * confirmed when a pull shows the server has applied it.
 */
export class LocalStore implements ReadTx {
  readonly #options: LocalStoreOptions;
  readonly #view: View;
  readonly #listeners = new Map<string, Set<Listener>>();
  readonly #clientId: string;
  #pending: PendingMutation[];
  #nextId: number;
  #cursor: string;
  #writes: Promise<void> = Promise.resolve();
  #syncing: Promise<SyncOutcome> | undefined;
  #version = 0;

  private constructor(
    options: LocalStoreOptions,
    state: {
      clientId: string;
      pending: PendingMutation[];
      nextId: number;
      cursor: string;
      view: View;
    },
  ) {
    this.#options = options;
    this.#clientId = state.clientId;
    this.#pending = state.pending;
    this.#nextId = state.nextId;
    this.#cursor = state.cursor;
    this.#view = state.view;
  }

  /** Loads the store from disk. Works offline: it never touches the network. */
  static async open(options: LocalStoreOptions): Promise<LocalStore> {
    const snapshot = await readSnapshot(options.database);
    const view = new View(options.mutators);
    view.load(snapshot.patches);
    const pending = [...snapshot.pending].toSorted((a, b) => a.id - b.id);
    view.rebase([], false, pending);
    let clientId = snapshot.meta.get("clientId");
    if (clientId === undefined) {
      clientId = crypto.randomUUID();
      await writeMeta(options.database, { clientId });
    }
    const confirmed = Number(snapshot.meta.get("confirmed") ?? "0");
    const nextId =
      Math.max(confirmed, ...pending.map((mutation) => mutation.id)) + 1;
    const cursor = snapshot.meta.get("cursor") ?? "";
    return new LocalStore(options, { clientId, pending, nextId, cursor, view });
  }

  /** This device's identity for the server's idempotency table. */
  get clientId(): string {
    return this.#clientId;
  }

  /** Mutations not yet confirmed by the server. */
  get pending(): readonly PendingMutation[] {
    return this.#pending;
  }

  /** Increases on every visible change; a snapshot key for React. */
  get version(): number {
    return this.#version;
  }

  /** Reads one record. */
  get(collection: string, key: string): JsonValue | undefined {
    return this.#view.get(collection, key);
  }

  /** Reads every live record of a collection, in key order. */
  scan(collection: string): readonly (readonly [string, JsonValue])[] {
    return this.#view.scan(collection);
  }

  /**
   * Calls `listener` whenever a record in one of `collections` may have
   * changed. Returns the unsubscribe function.
   */
  subscribe(collections: readonly string[], listener: Listener): () => void {
    for (const collection of collections) {
      const set = this.#listeners.get(collection) ?? new Set();
      set.add(listener);
      this.#listeners.set(collection, set);
    }
    return () => {
      for (const collection of collections)
        this.#listeners.get(collection)?.delete(listener);
    };
  }

  /**
   * Applies a mutation now and queues it for the server.
   *
   * @throws when the mutator is unknown or throws; nothing is queued then
   */
  mutate(name: string, input: JsonValue): PendingMutation {
    const started = performance.now();
    const mutation: PendingMutation = {
      id: this.#nextId,
      name,
      args: input,
      createdAt: (this.#options.now ?? Date.now)(),
    };
    const touched = this.#view.apply(mutation);
    this.#nextId += 1;
    this.#pending.push(mutation);
    this.#notify(touched);
    telemetry().histogram(
      "vellum.client.mutation.perceived_ms",
      performance.now() - started,
      {
        mutator: name,
      },
    );
    this.#persist(() => enqueue(this.#options.database, mutation));
    return mutation;
  }

  /** Resolves once every local change so far is on disk. */
  async flushed(): Promise<void> {
    await this.#writes;
  }

  /**
   * Pushes pending mutations and pulls the server's changes. Concurrent
   * calls share one round trip.
   *
   * @throws SyncError from the transport; the store is unchanged then
   */
  async sync(): Promise<SyncOutcome> {
    this.#syncing ??= this.#roundTrip().finally(() => {
      this.#syncing = undefined;
    });
    return this.#syncing;
  }

  async #roundTrip(): Promise<SyncOutcome> {
    const started = performance.now();
    await this.flushed();
    const batch = [...this.#pending];
    if (batch.length > 0) {
      const pushed = await this.#options.transport.push({
        clientId: this.#clientId,
        mutations: batch,
      });
      this.#reject(pushed.rejected);
    }
    let pulled = 0;
    let more = true;
    while (more) {
      const pull = await this.#options.transport.pull(
        this.#clientId,
        this.#cursor,
      );
      this.#pending = this.#pending.filter(
        (mutation) => mutation.id > pull.lastMutationId,
      );
      this.#cursor = pull.cursor;
      const touched = this.#view.rebase(
        pull.patches,
        pull.reset,
        this.#pending,
      );
      this.#persist(() =>
        commitPull(this.#options.database, pull, pull.lastMutationId),
      );
      this.#notify(touched);
      pulled += pull.patches.length;
      more = pull.more === true;
    }
    telemetry().histogram(
      "vellum.client.sync.round_trip_ms",
      performance.now() - started,
      {
        pushed: batch.length,
      },
    );
    await this.flushed();
    return { pushed: batch.length, pulled, pending: this.#pending.length };
  }

  #reject(rejected: readonly RejectedMutation[]): void {
    if (rejected.length === 0) return;
    const ids = new Set(rejected.map((entry) => entry.id));
    for (const entry of rejected) {
      telemetry().count("vellum.client.mutation.rejected", {
        code: entry.code,
      });
      this.#options.onRejected?.(
        entry,
        this.#pending.find((mutation) => mutation.id === entry.id),
      );
    }
    this.#pending = this.#pending.filter((mutation) => !ids.has(mutation.id));
    this.#persist(() => dropMutations(this.#options.database, [...ids]));
  }

  #persist(write: () => Promise<void>): void {
    // Keep the chain alive after a failure so later writes still run.
    this.#writes = this.#writes.then(write).catch((error: unknown) => {
      telemetry().count("vellum.client.store.disk_error");
      this.#options.onDiskError?.(error);
    });
  }

  #notify(collections: ReadonlySet<string>): void {
    this.#version += 1;
    const called = new Set<Listener>();
    for (const collection of collections) {
      for (const listener of this.#listeners.get(collection) ?? []) {
        if (called.has(listener)) continue;
        called.add(listener);
        listener();
      }
    }
  }
}
