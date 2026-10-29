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

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { createAuthApi } from "./api/auth";
import { createHttp } from "./api/http";
import { App } from "./app";
import { createHttpCollectionsStore } from "./collections/http";
import { createMockCollectionsStore } from "./collections/mock";
import { createNorthstarStore } from "./data/mock/northstar";
import { lazyCache } from "./editor/collab/lazy-cache";
import { createLocalConnector } from "./editor/collab/local-connector";
import { createBrowserPushDevice } from "./notifications/device";
import { createHttpNotificationPreferencesStore } from "./notifications/preferences";
import { createPlatform, startPlatform } from "./platform/platform";
import { DocumentCache } from "./platform/store/document-cache";
import { openLocalDatabase } from "./platform/store/persistence";

// Pages keep their edits across reloads in the browser's local database.
const cache = lazyCache(async () => new DocumentCache(await openLocalDatabase("vellum-northstar")));

const container = document.querySelector("#root");
if (container === null) {
  throw new Error("Vellum: the #root element is missing from index.html");
}
const root = createRoot(container);

// When a Vellum server answers, the areas whose API exists use it (issue
// #392); without one (the dev server alone, the website's demo) the whole
// app runs on the Northstar demo data.
const http = createHttp();
const auth = createAuthApi(http);
let status: Awaited<ReturnType<typeof auth.status>>;
try {
  status = await auth.status();
} catch {
  // A server that answers with an error: show the demo rather than nothing.
  status = undefined;
}
root.render(
  <StrictMode>
    <App
      store={createNorthstarStore()}
      collab={createLocalConnector({ cache })}
      collections={
        status === undefined ? createMockCollectionsStore() : createHttpCollectionsStore(http)
      }
      session={status === undefined ? undefined : { auth, status }}
      notifications={
        status === undefined
          ? undefined
          : {
              store: createHttpNotificationPreferencesStore(http),
              device: createBrowserPushDevice(),
            }
      }
    />
  </StrictMode>,
);

void startPlatform(createPlatform(), { production: import.meta.env.PROD });
