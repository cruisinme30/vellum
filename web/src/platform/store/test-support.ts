// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A fake server and shared mutators for the store tests. It implements the
 * push/pull contract with per-client idempotency, the way the real server
 * must.
 */

import type { SyncTransport } from "./transport";
import type {
  JsonValue,
  Mutators,
  Patch,
  PendingMutation,
  PullResponse,
  PushRequest,
  PushResponse,
} from "./types";
import { View } from "./view";

/** Mutators of a tiny issue tracker used across the tests. */
export const issueMutators: Mutators = {
  createIssue(tx, input) {
    const { key, title } = input as { key: string; title: string };
    tx.put("issues", key, { title, state: "backlog" });
  },
  setState(tx, input) {
    const { key, state } = input as { key: string; state: string };
    const issue = tx.get("issues", key) as Record<string, JsonValue> | undefined;
    if (issue === undefined) throw new Error(`no issue ${key}`);
    tx.put("issues", key, { ...issue, state });
  },
  deleteIssue(tx, input) {
    tx.del("issues", (input as { key: string }).key);
  },
};

/** An in-memory server with a change log. */
export class FakeServer implements SyncTransport {
  readonly view = new View(issueMutators);
  readonly log: Patch[] = [];
  readonly applied = new Map<string, number>();
  /** Mutation names the server refuses. */
  readonly forbidden = new Set<string>();
  pushes = 0;
  failNext: Error | undefined;

  push(request: PushRequest): Promise<PushResponse> {
    this.pushes += 1;
    const failure = this.#takeFailure();
    if (failure !== undefined) return Promise.reject(failure);
    const rejected = [];
    for (const mutation of request.mutations) {
      const last = this.applied.get(request.clientId) ?? 0;
      if (mutation.id <= last) continue;
      if (this.forbidden.has(mutation.name)) {
        rejected.push({ id: mutation.id, code: "forbidden", message: "not allowed" });
      } else {
        this.serverApply(mutation);
      }
      this.applied.set(request.clientId, mutation.id);
    }
    return Promise.resolve({ lastMutationId: this.applied.get(request.clientId) ?? 0, rejected });
  }

  pull(clientId: string, cursor: string): Promise<PullResponse> {
    const failure = this.#takeFailure();
    if (failure !== undefined) return Promise.reject(failure);
    const from = cursor === "" ? 0 : Number(cursor);
    return Promise.resolve({
      cursor: String(this.log.length),
      lastMutationId: this.applied.get(clientId) ?? 0,
      reset: false,
      patches: this.log.slice(from),
    });
  }

  /** Applies a mutation as the server, recording the resulting patches. */
  serverApply(mutation: Pick<PendingMutation, "name" | "args">): void {
    const before = new Map(this.view.scan("issues"));
    this.view.apply(mutation);
    const after = new Map(this.view.scan("issues"));
    for (const [key, value] of after) {
      if (before.get(key) !== value) this.log.push({ op: "put", collection: "issues", key, value });
    }
    for (const key of before.keys()) {
      if (!after.has(key)) this.log.push({ op: "del", collection: "issues", key });
    }
  }

  #takeFailure(): Error | undefined {
    const failure = this.failNext;
    this.failNext = undefined;
    return failure;
  }
}

let databaseCounter = 0;

/** A unique database name per test. */
export function freshDatabaseName(): string {
  databaseCounter += 1;
  return `vellum-test-${String(databaseCounter)}`;
}
