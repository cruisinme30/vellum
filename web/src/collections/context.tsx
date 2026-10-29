// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, useEffect, useState, type ReactNode } from "react";

import type { CollectionsStore } from "./store";

const CollectionsContext = createContext<CollectionsStore | undefined>(undefined);

/** Provides the collections store to the app. */
export function CollectionsProvider({
  store,
  children,
}: {
  readonly store: CollectionsStore;
  readonly children: ReactNode;
}) {
  return <CollectionsContext value={store}>{children}</CollectionsContext>;
}

/** The collections store; throws outside a provider. */
export function useCollections(): CollectionsStore {
  const store = use(CollectionsContext);
  if (!store) {
    throw new Error("Vellum: useCollections needs a CollectionsProvider");
  }
  return store;
}

/** The state of an asynchronous load. */
export interface Loaded<T> {
  readonly data: T | undefined;
  readonly error: string | undefined;
}

/**
 * Loads data from the store and loads it again after every write. `key`
 * names what is loaded: a new key starts over. The previous data stays on
 * screen while a reload is in flight, so views do not flicker.
 */
export function useLoad<T>(key: string, load: (store: CollectionsStore) => Promise<T>): Loaded<T> {
  const store = useCollections();
  const [state, setState] = useState<{ key: string } & Loaded<T>>({
    key,
    data: undefined,
    error: undefined,
  });
  const [version, setVersion] = useState(0);
  useEffect(
    () =>
      store.subscribe(() => {
        setVersion((v) => v + 1);
      }),
    [store],
  );
  useEffect(() => {
    let live = true;
    load(store).then(
      (data) => {
        if (live) {
          setState({ key, data, error: undefined });
        }
      },
      (error: unknown) => {
        if (live) {
          setState({ key, data: undefined, error: messageOf(error) });
        }
      },
    );
    return () => {
      live = false;
    };
    // `load` is a fresh closure every render; `key` names what it loads.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, key, version]);
  return state.key === key ? state : { data: undefined, error: undefined };
}

/** The message of whatever a store call rejected with. */
export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
