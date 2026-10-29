// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import "@fontsource/roboto/400.css";
import "@fontsource/roboto/500.css";
import "@fontsource/roboto/700.css";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/editor.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app";
import { createNorthstarStore } from "./data/mock/northstar";
import { createLocalConnector } from "./editor/collab/local-connector";

const container = document.querySelector("#root");
if (container === null) {
  throw new Error("Vellum: the #root element is missing from index.html");
}
createRoot(container).render(
  <StrictMode>
    <App store={createNorthstarStore()} collab={createLocalConnector()} />
  </StrictMode>,
);
