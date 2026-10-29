// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "@fontsource/roboto/400.css";
import "@fontsource/roboto/500.css";
import "@fontsource/roboto/700.css";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/editor.css";
import "./styles/collections.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app";
import { createMockCollectionsStore } from "./collections/mock";
import { createNorthstarStore } from "./data/mock/northstar";
import { lazyCache } from "./editor/collab/lazy-cache";
import { createLocalConnector } from "./editor/collab/local-connector";
import { createPlatform, startPlatform } from "./platform/platform";
import { DocumentCache } from "./platform/store/document-cache";
import { openLocalDatabase } from "./platform/store/persistence";

// Pages keep their edits across reloads in the browser's local database.
const cache = lazyCache(async () => new DocumentCache(await openLocalDatabase("vellum-northstar")));

const container = document.querySelector("#root");
if (container === null) {
  throw new Error("Vellum: the #root element is missing from index.html");
}
createRoot(container).render(
  <StrictMode>
    <App
      store={createNorthstarStore()}
      collab={createLocalConnector({ cache })}
      collections={createMockCollectionsStore()}
    />
  </StrictMode>,
);

void startPlatform(createPlatform(), { production: import.meta.env.PROD });
