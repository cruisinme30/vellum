// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "@fontsource/roboto/400.css";
import "@fontsource/roboto/500.css";
import "@fontsource/roboto/700.css";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/editor.css";
import "./styles/collections.css";
import "./styles/notifications.css";
import "./styles/appearance.css";
import "./styles/render.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { createHttpAdminConsoleApi } from "./admin/console/api";
import { createAuthApi } from "./api/auth";
import { createHttp } from "./api/http";
import { createReauthGate, withReauth } from "./api/reauth";
import { App } from "./app";
import { createHttpAttachmentsApi } from "./attachments/api";
import { createHttpBooksApi } from "./books/api";
import { createHttpCollectionsStore } from "./collections/http";
import { createMockCollectionsStore } from "./collections/mock";
import { createHttpPageHistory } from "./data/history";
import { createHttpWorkspaceStore } from "./data/http";
import { createNorthstarStore } from "./data/mock/northstar";
import { createHttpDriveApi } from "./drive/api";
import { lazyCache } from "./editor/collab/lazy-cache";
import { createLocalConnector } from "./editor/collab/local-connector";
import { createSocketConnector } from "./editor/collab/socket-connector";
import { createBrowserPushDevice } from "./notifications/device";
import { createHttpNotificationPreferencesStore } from "./notifications/preferences";
import {
  combineDirectories,
  createHttpDirectory,
  createHttpGroupDirectory,
} from "./people/directory";
import { startClientTelemetry } from "./platform/client-telemetry-start";
import { createPlatform, startPlatform } from "./platform/platform";
import { createHttpPrivateSpacesApi } from "./platform/private/api";
import { DocumentCache } from "./platform/store/document-cache";
import { openLocalDatabase } from "./platform/store/persistence";
import { createHttpRenderApi } from "./render/api";
import { createHttpOrganisationAppearance } from "./theme/organisation";
import { createHttpWorkApi } from "./work/api";

// Demo pages keep their edits across reloads in the browser's local
// database. With a server, the page's room on /collab is the store.
const cache = lazyCache(async () => new DocumentCache(await openLocalDatabase("vellum-northstar")));

const container = document.querySelector("#root");
if (container === null) {
  throw new Error("Vellum: the #root element is missing from index.html");
}
startClientTelemetry({
  performance,
  location,
  document,
  Observer: globalThis.PerformanceObserver,
});
const root = createRoot(container);

// When a Vellum server answers, the areas whose API exists use it (issue
// #392); without one (the dev server alone, the website's demo) the whole
// app runs on the Northstar demo data.
const raw = createHttp();
const auth = createAuthApi(raw);
// Every other call asks the person to confirm who they are when the
// server wants a recent sign-in, then runs again (D241).
const gate = createReauthGate();
const http = withReauth(raw, gate);
// Roboto's heavier faces download only once text uses them, so the first
// paint would set headings and links in the fallback font and then reflow
// (a layout shift). Fetch them while the server answers, for at most a
// moment: a slow font still shows its fallback rather than holding the page.
async function preloadFonts(): Promise<void> {
  try {
    await Promise.race([
      Promise.all(["500 1em Roboto", "700 1em Roboto"].map((font) => document.fonts.load(font))),
      new Promise((resolve) => setTimeout(resolve, 800)),
    ]);
  } catch {
    // A font that fails to load keeps its fallback.
  }
}
async function loadStatus(): Promise<Awaited<ReturnType<typeof auth.status>>> {
  try {
    return await auth.status();
  } catch {
    // A server that answers with an error: show the demo rather than nothing.
    return undefined;
  }
}
const [status] = await Promise.all([loadStatus(), preloadFonts()]);
root.render(
  <StrictMode>
    <App
      store={
        status === undefined
          ? createNorthstarStore()
          : createHttpWorkspaceStore(http, { user: status.user ?? undefined })
      }
      collab={status === undefined ? createLocalConnector({ cache }) : createSocketConnector()}
      history={status === undefined ? undefined : createHttpPageHistory(http)}
      admin={status === undefined ? undefined : createHttpAdminConsoleApi(http)}
      work={status === undefined ? undefined : createHttpWorkApi(http)}
      books={status === undefined ? undefined : createHttpBooksApi(http)}
      drive={status === undefined ? undefined : createHttpDriveApi(http)}
      directory={
        status === undefined
          ? undefined
          : combineDirectories(createHttpDirectory(http), createHttpGroupDirectory(http))
      }
      attachments={status === undefined ? undefined : createHttpAttachmentsApi(http)}
      privateSpaces={status === undefined ? undefined : createHttpPrivateSpacesApi(http)}
      render={status === undefined ? undefined : createHttpRenderApi(http)}
      collections={
        status === undefined ? createMockCollectionsStore() : createHttpCollectionsStore(http)
      }
      session={status === undefined ? undefined : { auth, status }}
      appearance={status === undefined ? undefined : createHttpOrganisationAppearance(http)}
      reauth={status === undefined ? undefined : { gate, http: raw }}
      notifications={
        status === undefined
          ? undefined
          : {
              store: createHttpNotificationPreferencesStore(http),
              device: createBrowserPushDevice("", http),
            }
      }
    />
  </StrictMode>,
);

void startPlatform(createPlatform(), { production: import.meta.env.PROD });
