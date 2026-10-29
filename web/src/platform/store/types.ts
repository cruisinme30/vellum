// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Types shared by the local-first store and its sync wire contract (D98).
 *
 * The contract is push/pull: the client pushes its pending mutations in
 * order, the server applies each one at most once (keyed by client ID and
 * mutation ID), and the client pulls patches since its last cursor.
 */

/** A JSON value as stored in a record. Records are treated as immutable. */
export type JsonValue =
  string | number | boolean | null | readonly JsonValue[] | { readonly [key: string]: JsonValue };

/** One mutation waiting to be confirmed by the server. */
export interface PendingMutation {
  /** Monotonic per client, starting at 1. The server's idempotency key. */
  readonly id: number;
  /** Mutator name, the same on client and server. */
  readonly name: string;
  /** Mutator arguments. */
  readonly args: JsonValue;
  /** Wall-clock time the user made the change, epoch milliseconds. */
  readonly createdAt: number;
}

/** Body of `POST /api/v1/sync/push`. */
export interface PushRequest {
  readonly clientId: string;
  readonly mutations: readonly PendingMutation[];
}

/** A mutation the server refused, for example on a permission check. */
export interface RejectedMutation {
  readonly id: number;
  /** Stable error code from the kernel's `ErrorKind`. */
  readonly code: string;
  readonly message: string;
}

/** Response to a push. */
export interface PushResponse {
  /** Every mutation up to and including this ID has been processed. */
  readonly lastMutationId: number;
  readonly rejected: readonly RejectedMutation[];
}

/** A change to one record. */
export type Patch =
  | {
      readonly op: "put";
      readonly collection: string;
      readonly key: string;
      readonly value: JsonValue;
    }
  | { readonly op: "del"; readonly collection: string; readonly key: string };

/** Response to `GET /api/v1/sync/pull?clientId=&cursor=`. */
export interface PullResponse {
  /** Opaque; sent back on the next pull. */
  readonly cursor: string;
  /** The last mutation of this client reflected in `patches`. */
  readonly lastMutationId: number;
  /** When true the client drops its base state before applying `patches`. */
  readonly reset: boolean;
  readonly patches: readonly Patch[];
  /** When true, more patches are waiting: pull again with the new cursor. */
  readonly more?: boolean;
}

/** Read access inside a mutator or a query. */
export interface ReadTx {
  get(collection: string, key: string): JsonValue | undefined;
  /** Every live record of a collection, in key order. */
  scan(collection: string): readonly (readonly [string, JsonValue])[];
}

/** Write access inside a mutator. */
export interface WriteTx extends ReadTx {
  put(collection: string, key: string, value: JsonValue): void;
  del(collection: string, key: string): void;
}

/**
 * A named change. It must be deterministic: the store replays pending
 * mutations on top of fresh server state, and the server runs its own
 * implementation of the same name.
 */
type Mutator = (tx: WriteTx, input: JsonValue) => void;

/** The mutators an application registers, by name. */
export type Mutators = Readonly<Record<string, Mutator>>;
