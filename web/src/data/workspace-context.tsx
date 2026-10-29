// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, useCallback, useSyncExternalStore, type ReactNode } from "react";

import type { CollabConnector } from "../editor/collab/provider";

import type { WorkspaceSnapshot, WorkspaceStore } from "./store";

interface Services {
  readonly store: WorkspaceStore;
  readonly collab: CollabConnector;
}

const ServicesContext = createContext<Services | undefined>(undefined);

/** Provides the workspace store and the collab connector to the app. */
export function WorkspaceProvider({
  store,
  collab,
  children,
}: Services & { readonly children: ReactNode }) {
  return <ServicesContext value={{ store, collab }}>{children}</ServicesContext>;
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
  const subscribe = useCallback((listener: () => void) => store.subscribe(listener), [store]);
  const snapshot = useCallback(() => store.getSnapshot(), [store]);
  return useSyncExternalStore(subscribe, snapshot);
}
