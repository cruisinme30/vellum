// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useRef, useSyncExternalStore } from "react";

import type { LocalStore } from "./local-store";
import type { ReadTx } from "./types";

/**
 * Runs `query` against the store and re-renders when a record in one of
 * `collections` changes. The query runs synchronously on memory, so the
 * first render already has data and a local edit re-renders in the same
 * frame.
 *
 * @param collections - must list every collection the query reads
 */
export function useQuery<T>(
  store: LocalStore,
  collections: readonly string[],
  query: (tx: ReadTx) => T,
): T {
  const key = collections.join("\u0000");
  const cache = useRef<
    | { version: number; key: string; query: (tx: ReadTx) => T; value: T }
    | undefined
  >(undefined);
  const subscribe = useCallback(
    (listener: () => void) => store.subscribe(key.split("\u0000"), listener),
    [store, key],
  );
  const snapshot = (): T => {
    const cached = cache.current;
    if (
      cached?.version === store.version &&
      cached.key === key &&
      cached.query === query
    ) {
      return cached.value;
    }
    const value = query(store);
    cache.current = { version: store.version, key, query, value };
    return value;
  };
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
