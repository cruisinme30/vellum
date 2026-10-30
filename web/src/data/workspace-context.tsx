// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import type { CollabConnector } from "../editor/collab/provider";

import type { PageHistory } from "./history";
import type { WorkspaceSnapshot, WorkspaceStore } from "./store";
import type { SearchFilters, SearchHit } from "./types";

interface Services {
  readonly store: WorkspaceStore;
  readonly collab: CollabConnector;
  /** Saved versions of pages; only a Vellum server keeps them. */
  readonly history?: PageHistory | undefined;
}

const ServicesContext = createContext<Services | undefined>(undefined);

/**
 * Provides the workspace store, the collab connector and page history to the
 * app.
 */
export function WorkspaceProvider({
  store,
  collab,
  history,
  children,
}: Services & { readonly children: ReactNode }) {
  const services = useMemo(
    () => ({ store, collab, history }),
    [store, collab, history],
  );
  return <ServicesContext value={services}>{children}</ServicesContext>;
}

/** The services; throws outside a provider. */
export function useServices(): Services {
  const services = use(ServicesContext);
  if (!services) {
    throw new Error("Vellum: useServices needs a WorkspaceProvider");
  }
  return services;
}

/** The current workspace snapshot; re-renders when it changes. */
export function useWorkspace(): WorkspaceSnapshot {
  const { store } = useServices();
  const subscribe = useCallback(
    (listener: () => void) => store.subscribe(listener),
    [store],
  );
  const snapshot = useCallback(() => store.getSnapshot(), [store]);
  return useSyncExternalStore(subscribe, snapshot);
}

/**
 * The pages that match a search (`kind: "search"`) or mention a handle
 * (`kind: "mentions"`), asked again whenever the workspace changes;
 * `undefined` until the first answer for this text arrives. Keep `filters`
 * stable (memoised): a new object asks again.
 */
export function useSearch(
  kind: "search" | "mentions",
  text: string,
  filters?: SearchFilters,
): readonly SearchHit[] | undefined {
  const { store } = useServices();
  const workspace = useWorkspace();
  const key = `${kind}:${text}:${JSON.stringify(filters ?? {})}`;
  const [answer, setAnswer] = useState<{ key: string; hits: SearchHit[] }>();
  useEffect(() => {
    let current = true;
    const asked =
      kind === "search" ? store.search(text, filters) : store.mentions(text);
    void asked.then((hits) => {
      if (current) setAnswer({ key, hits });
    });
    return () => {
      current = false;
    };
  }, [store, kind, text, filters, key, workspace]);
  return answer?.key === key ? answer.hits : undefined;
}
