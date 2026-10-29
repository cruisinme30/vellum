// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./app";
import { startClientTelemetry } from "./platform/client-telemetry-start";

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
createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
