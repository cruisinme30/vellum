// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { lazy, Suspense, useEffect, useMemo, useState, type ReactNode } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

import type { AdminConsoleApi } from "./admin/console/api";
import { AuditPage } from "./admin/console/audit-page";
import { BackupsPage } from "./admin/console/backups-page";
import { AdminConsoleProvider } from "./admin/console/context";
import { SettingsPage } from "./admin/console/settings-page";
import { RequestsPage } from "./admin/requests/requests-page";
import { RoutesPage } from "./admin/routes/routes-page";
import type { AuthApi, AuthStatus } from "./api/auth";
import { RequireSession, SessionProvider, useSession } from "./api/session";
import type { BooksApi } from "./books/api";
import { BookPage, BooksPage } from "./books/books-page";
import { CollectionView } from "./collections/collection-view";
import { CollectionsIndexView } from "./collections/collections-index";
import { CollectionsProvider } from "./collections/context";
import type { CollectionsStore } from "./collections/store";
import type { PageHistory } from "./data/history";
import { ServerApisProvider } from "./data/server-apis";
import type { WorkspaceStore } from "./data/store";
import { useServices, WorkspaceProvider } from "./data/workspace-context";
import type { CollabConnector } from "./editor/collab/provider";
import { I18nProvider, useT } from "./i18n/i18n";
import { KeyboardRoot } from "./keyboard/keyboard-root";
import { NotificationsProvider, type NotificationServices } from "./notifications/context";
import { NotificationSettingsView } from "./notifications/notification-settings";
import { createMockNotificationPreferencesStore } from "./notifications/preferences";
import type { PrivateSpacesApi } from "./platform/private/api";
import { PrivateSpacesView } from "./platform/private/private-spaces-view";
import { AppShell } from "./shell/app-shell";
import { AppearanceProvider } from "./theme/appearance-context";
import { FavoritesView, HomeView } from "./views/home";
import { AdminView, HelpView, NotFoundView } from "./views/info";
import { NewPageView } from "./views/new-page";
import { InboxView, SearchView } from "./views/search";
import { SetupView, SignInView } from "./views/sign-in";
import type { WorkApi } from "./work/api";
import { IssuePage } from "./work/issue-page";
import { IssuesPage } from "./work/issues-page";
import { MyWorkPage } from "./work/my-work-page";
import type { PlanningApi } from "./work/planning/api";
import { AskPage, AsksPage } from "./work/planning/asks-page";
import { InitiativesPage } from "./work/planning/initiatives-page";
import { InsightsPage } from "./work/planning/insights-page";
import { NewIssuePage } from "./work/planning/new-issue-page";
import { ProjectPage } from "./work/planning/project-page";
import { ProjectsPage } from "./work/planning/projects-page";

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
        <Route path="settings/notifications" element={<NotificationSettingsView />} />
        <Route path="settings/private-spaces" element={<PrivateSpacesView />} />
        <Route path="new" element={<NewPageView />} />
        <Route path="my-work" element={<MyWorkPage />} />
        <Route path="issues" element={<IssuesPage />} />
        {/* Work planning (D223-D225); /issues/new before /issues/:issueKey. */}
        <Route path="issues/new" element={<NewIssuePage />} />
        <Route path="projects" element={<ProjectsPage />} />
        <Route path="projects/:projectId" element={<ProjectPage />} />
        <Route path="initiatives" element={<InitiativesPage />} />
        <Route path="insights" element={<InsightsPage />} />
        <Route path="asks" element={<AsksPage />} />
        <Route path="asks/:team/:slug" element={<AskPage />} />
        <Route path="issues/:issueKey" element={<IssuePage />} />
        <Route path="books" element={<BooksPage />} />
        <Route path="books/:bookId" element={<BookPage />} />
        <Route path="books/:bookId/:sectionId" element={<BookPage />} />
        <Route path="data" element={<CollectionsIndexView />} />
        <Route path="data/:collectionId" element={<CollectionView />} />
        <Route path="help" element={<HelpView />} />
        <Route path="admin" element={<AdminView />} />
        <Route path="admin/requests" element={<RequestsPage />} />
        <Route path="admin/routes" element={<RoutesPage />} />
        <Route path="admin/audit" element={<AuditPage />} />
        <Route path="admin/settings" element={<SettingsPage />} />
        <Route path="admin/backups" element={<BackupsPage />} />
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
  /** Saved page versions, from a Vellum server; absent in demo mode. */
  readonly history?: PageHistory | undefined;
  /** The admin console's audit, settings and backups; absent in demo mode. */
  readonly admin?: AdminConsoleApi | undefined;
  /** My Work and issues, from a Vellum server; absent in demo mode. */
  readonly work?: WorkApi | undefined;
  /** Books, from a Vellum server; absent in demo mode. */
  readonly books?: BooksApi | undefined;
  /** Private spaces' device keys, from a Vellum server; absent in demo mode. */
  readonly privateSpaces?: PrivateSpacesApi | undefined;
  /** Work planning, from a Vellum server; absent in demo mode. */
  readonly planning?: PlanningApi | undefined;
  readonly collections: CollectionsStore;
  /** Present when a Vellum server answered; absent in demo mode. */
  readonly session?: { readonly auth: AuthApi; readonly status: AuthStatus } | undefined;
  /** Notification settings; the Northstar demo's when absent. */
  readonly notifications?: NotificationServices | undefined;
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
  history,
  admin,
  work,
  books,
  privateSpaces,
  planning,
  collections,
  session,
  notifications,
  children,
}: AppServices & { readonly children: ReactNode }) {
  const serverApis = useMemo(
    () => ({ work, books, privateSpaces, planning }),
    [work, books, privateSpaces, planning],
  );
  const [demoNotifications] = useState<NotificationServices>(() => ({
    store: createMockNotificationPreferencesStore(),
  }));
  return (
    <I18nProvider>
      <AppearanceProvider>
        <MaybeSession session={session}>
          <WorkspaceProvider store={store} collab={collab} history={history}>
            <SignedInUser>
              <CollectionsProvider store={collections}>
                <NotificationsProvider services={notifications ?? demoNotifications}>
                  <AdminConsoleProvider api={admin}>
                    <ServerApisProvider apis={serverApis}>{children}</ServerApisProvider>
                  </AdminConsoleProvider>
                </NotificationsProvider>
              </CollectionsProvider>
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
