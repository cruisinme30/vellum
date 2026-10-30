// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import react from "@vitejs/plugin-react";
import type { Connect, Plugin } from "vite";
import { defineConfig } from "vitest/config";

import { inlineAsset } from "./src/platform/inline-assets";
import { vellumServiceWorker } from "./src/platform/pwa/vite-plugin";

// `VELLUM_SERVER=http://127.0.0.1:8080 pnpm dev` runs the app against a
// local `vellum serve`; without it the app runs on the Northstar demo data.
const server = process.env["VELLUM_SERVER"];
const proxy =
  server === undefined
    ? undefined
    : Object.fromEntries(
        ["/api", "/auth", "/oauth", "/saml", "/.well-known", "/collab"].map((path) => [
          path,
          { target: server, changeOrigin: false, ws: path === "/collab" },
        ]),
      );

/**
 * Without a server, the dev and preview servers answer the app's
 * "is a Vellum server here?" check themselves, so the demo starts without
 * a failed request in the console (the UI suite counts those as errors).
 */
const answerDemoStatus: Connect.NextHandleFunction = (request, response, next) => {
  if (request.url?.split("?")[0] !== "/api/v1/auth/status") {
    next();
    return;
  }
  response.setHeader("content-type", "application/json");
  response.end(JSON.stringify({ demo: true }));
};

const demoMode = (): Plugin => ({
  name: "vellum-demo-mode",
  configureServer(developmentServer) {
    developmentServer.middlewares.use(answerDemoStatus);
  },
  configurePreviewServer(previewServer) {
    previewServer.middlewares.use(answerDemoStatus);
  },
});

export default defineConfig({
  plugins: [react(), vellumServiceWorker(), ...(proxy === undefined ? [demoMode()] : [])],
  ...(proxy === undefined ? {} : { server: { proxy } }),
  build: {
    sourcemap: true,
    // The editor chunk (Tiptap, ProseMirror, Yjs) is about 560 kB; it loads lazily.
    chunkSizeWarningLimit: 600,
    // Fonts stay files: the CSP blocks `data:` fonts (#569).
    assetsInlineLimit: inlineAsset,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/main.tsx", "src/test/**", "src/platform/pwa/sw.ts"],
      thresholds: { lines: 85, functions: 85, branches: 85, statements: 85 },
    },
  },
});
