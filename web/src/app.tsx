// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { lazy, Suspense, useEffect, type ReactNode } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

import type { AuthApi, AuthStatus } from "./api/auth";
import { RequireSession, SessionProvider, useSession } from "./api/session";
import { CollectionView } from "./collections/collection-view";
import { CollectionsIndexView } from "./collections/collections-index";
import { CollectionsProvider } from "./collections/context";
import type { CollectionsStore } from "./collections/store";
import type { WorkspaceStore } from "./data/store";
import { useServices, WorkspaceProvider } from "./data/workspace-context";
import type { CollabConnector } from "./editor/collab/provider";
import { I18nProvider, useT } from "./i18n/i18n";
import { KeyboardRoot } from "./keyboard/keyboard-root";
import { AppShell } from "./shell/app-shell";
import { AppearanceProvider } from "./theme/appearance-context";
import { FavoritesView, HomeView } from "./views/home";
import { AdminView, HelpView, NotFoundView } from "./views/info";
import { NewPageView } from "./views/new-page";
import { InboxView, SearchView } from "./views/search";
import { SetupView, SignInView } from "./views/sign-in";

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
      <Route path="sign-in" element={<SignInView />} />
      <Route path="setup" element={<SetupView />} />
      <Route
        element={
          <RequireSession>
            <AppShell />
          </RequireSession>
        }
      >
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

/**
 * Services the app runs on. Areas whose server API exists use it when a
 * Vellum server answers; the rest, and everything in demo mode, use the
 * Northstar demo adapters (issue #392).
 */
export interface AppServices {
  readonly store: WorkspaceStore;
  readonly collab: CollabConnector;
  readonly collections: CollectionsStore;
  /** Present when a Vellum server answered; absent in demo mode. */
  readonly session?: { readonly auth: AuthApi; readonly status: AuthStatus } | undefined;
}

function MaybeSession({
  session,
  children,
}: {
  readonly session: AppServices["session"];
  readonly children: ReactNode;
}) {
  return session === undefined ? (
    children
  ) : (
    <SessionProvider auth={session.auth} initial={session.status}>
      {children}
    </SessionProvider>
  );
}

/** Tells the workspace store who is signed in, so it loads their drive. */
function SignedInUser({ children }: { readonly children: ReactNode }) {
  const { store } = useServices();
  const session = useSession();
  const user = session?.status.user ?? undefined;
  const id = user?.id;
  const handle = user?.handle;
  const name = user?.name;
  useEffect(() => {
    if (session === undefined) return;
    store.setUser?.(
      id === undefined || handle === undefined || name === undefined
        ? undefined
        : { id, handle, name },
    );
  }, [store, session, id, handle, name]);
  return children;
}

/** Providers shared by the app and its tests. */
export function AppProviders({
  store,
  collab,
  collections,
  session,
  children,
}: AppServices & { readonly children: ReactNode }) {
  return (
    <I18nProvider>
      <AppearanceProvider>
        <MaybeSession session={session}>
          <WorkspaceProvider store={store} collab={collab}>
            <SignedInUser>
              <CollectionsProvider store={collections}>{children}</CollectionsProvider>
            </SignedInUser>
          </WorkspaceProvider>
        </MaybeSession>
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
