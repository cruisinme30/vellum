// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { lazy, Suspense } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

import { RequestsPage } from "./admin/requests/requests-page";
import { RoutesPage } from "./admin/routes/routes-page";
import type { WorkspaceStore } from "./data/store";
import { WorkspaceProvider } from "./data/workspace-context";
import type { CollabConnector } from "./editor/collab/provider";
import { I18nProvider, useT } from "./i18n/i18n";
import { KeyboardRoot } from "./keyboard/keyboard-root";
import { AppShell } from "./shell/app-shell";
import { AppearanceProvider } from "./theme/appearance-context";
import { FavoritesView, HomeView } from "./views/home";
import { AdminView, CollectionsView, HelpView, NotFoundView } from "./views/info";
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
        <Route path="data" element={<CollectionsView />} />
        <Route path="help" element={<HelpView />} />
        <Route path="admin" element={<AdminView />} />
        <Route path="admin/requests" element={<RequestsPage />} />
        <Route path="admin/routes" element={<RoutesPage />} />
        <Route path="*" element={<NotFoundView />} />
      </Route>
    </Routes>
  );
}

/** Services the app runs on: mock adapters today, the API when it lands. */
export interface AppServices {
  readonly store: WorkspaceStore;
  readonly collab: CollabConnector;
}

/** Providers shared by the app and its tests. */
export function AppProviders({
  store,
  collab,
  children,
}: AppServices & { readonly children: React.ReactNode }) {
  return (
    <I18nProvider>
      <AppearanceProvider>
        <WorkspaceProvider store={store} collab={collab}>
          {children}
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
