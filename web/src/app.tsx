// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { lazy, Suspense } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

import { CollectionView } from "./collections/collection-view";
import { CollectionsIndexView } from "./collections/collections-index";
import { CollectionsProvider } from "./collections/context";
import type { CollectionsStore } from "./collections/store";
import type { WorkspaceStore } from "./data/store";
import { WorkspaceProvider } from "./data/workspace-context";
import type { CollabConnector } from "./editor/collab/provider";
import { I18nProvider, useT } from "./i18n/i18n";
import { KeyboardRoot } from "./keyboard/keyboard-root";
import { AppShell } from "./shell/app-shell";
import { AppearanceProvider } from "./theme/appearance-context";
import { FavoritesView, HomeView } from "./views/home";
import { AdminView, HelpView, NotFoundView } from "./views/info";
import { NewPageView } from "./views/new-page";
import { InboxView, SearchView } from "./views/search";

// The editor is most of the bundle, so it loads with the first page opened.
const PageView = lazy(async () => {
  const view = await import("./views/page-view");
  return { default: view.PageView };
});

function PageLoading() {
  const t = useT();
  return (
    <p className="vl-muted" role="status">
      {t("editor-loading")}
    </p>
  );
}

/** Every route of the app, inside the shell. */
export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<HomeView />} />
        <Route
          path="p/:pageId"
          element={
            <Suspense fallback={<PageLoading />}>
              <PageView />
            </Suspense>
          }
        />
        <Route path="search" element={<SearchView />} />
        <Route path="favorites" element={<FavoritesView />} />
        <Route path="inbox" element={<InboxView />} />
        <Route path="new" element={<NewPageView />} />
        {/* My Work lands with the tasks service; until then, mentions are the closest view. */}
        <Route path="my-work" element={<InboxView />} />
        <Route path="data" element={<CollectionsIndexView />} />
        <Route path="data/:collectionId" element={<CollectionView />} />
        <Route path="help" element={<HelpView />} />
        <Route path="admin" element={<AdminView />} />
        <Route path="*" element={<NotFoundView />} />
      </Route>
    </Routes>
  );
}

/** Services the app runs on: mock adapters today, the API when it lands. */
export interface AppServices {
  readonly store: WorkspaceStore;
  readonly collab: CollabConnector;
  readonly collections: CollectionsStore;
}

/** Providers shared by the app and its tests. */
export function AppProviders({
  store,
  collab,
  collections,
  children,
}: AppServices & { readonly children: React.ReactNode }) {
  return (
    <I18nProvider>
      <AppearanceProvider>
        <WorkspaceProvider store={store} collab={collab}>
          <CollectionsProvider store={collections}>{children}</CollectionsProvider>
        </WorkspaceProvider>
      </AppearanceProvider>
    </I18nProvider>
  );
}

/** The Vellum web app. */
export function App(services: AppServices) {
  return (
    <AppProviders {...services}>
      <KeyboardRoot>
        <BrowserRouter>
          <AppRoutes />
        </BrowserRouter>
      </KeyboardRoot>
    </AppProviders>
  );
}
