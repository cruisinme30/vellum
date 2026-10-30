// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Renders the app (or part of it) with the Northstar store, in memory.
import { render, type RenderResult } from "@testing-library/react";
import { userEvent, type UserEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, useLocation } from "react-router";

import type { AuthApi, AuthStatus } from "../api/auth";
import { AppProviders, AppRoutes } from "../app";
import { createMockCollectionsStore } from "../collections/mock";
import type { CollectionsStore } from "../collections/store";
import { createNorthstarStore } from "../data/mock/northstar";
import type { WorkspaceStore } from "../data/store";
import {
  createLocalConnector,
  type ChannelFactory,
} from "../editor/collab/local-connector";

const noop = () => {
  // A stub method.
};

const silentChannel: ChannelFactory = () => ({
  postMessage: noop,
  close: noop,
  addEventListener: noop,
  removeEventListener: noop,
});

/** A collab connector whose tabs never hear from each other. */
export function createTestCollab() {
  return createLocalConnector({ channel: silentChannel });
}

/** Shows the current location, so tests can assert where navigation went. */
function LocationProbe() {
  const location = useLocation();
  return (
    <div hidden data-testid="location">
      {location.pathname}
      {location.search}
    </div>
  );
}

interface Rendered extends RenderResult {
  readonly store: WorkspaceStore;
  readonly collections: CollectionsStore;
  readonly user: UserEvent;
  /** The router's current path and query. */
  readonly location: () => string;
}

/** Renders any element inside the app providers and a memory router. */
export function renderWithProviders(
  element: ReactNode,
  path = "/",
  store: WorkspaceStore = createNorthstarStore(),
  collections: CollectionsStore = createMockCollectionsStore(),
  session?: { readonly auth: AuthApi; readonly status: AuthStatus },
): Rendered {
  const collab = createTestCollab();
  const user = userEvent.setup();
  const result = render(
    <AppProviders
      store={store}
      collab={collab}
      collections={collections}
      session={session}
    >
      <MemoryRouter initialEntries={[path]}>
        {element}
        <LocationProbe />
      </MemoryRouter>
    </AppProviders>,
  );
  return {
    ...result,
    store,
    collections,
    user,
    location: () => result.getByTestId("location").textContent,
  };
}

/**
 * Renders every route of the app at a path, by default on the Northstar demo.
 */
export function renderApp(
  path = "/",
  store?: WorkspaceStore,
  collections?: CollectionsStore,
  session?: { readonly auth: AuthApi; readonly status: AuthStatus },
): Rendered {
  return renderWithProviders(<AppRoutes />, path, store, collections, session);
}

/** Makes `matchMedia` report a match for the given queries only. */
export function matchQueries(...queries: readonly string[]) {
  return (query: string) => ({
    matches: queries.includes(query),
    media: query,
    onchange: undefined,
    addEventListener: noop,
    removeEventListener: noop,
    addListener: noop,
    removeListener: noop,
    dispatchEvent: () => false,
  });
}
