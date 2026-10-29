// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { vellumServiceWorker } from "./src/platform/pwa/vite-plugin";

// `VELLUM_SERVER=http://127.0.0.1:8080 pnpm dev` runs the app against a
// local `vellum serve`; without it the app runs on the Northstar demo data.
const server = process.env["VELLUM_SERVER"];
const proxy =
  server === undefined
    ? undefined
    : Object.fromEntries(
        ["/api", "/auth", "/oauth", "/.well-known", "/collab"].map((path) => [
          path,
          { target: server, changeOrigin: false, ws: path === "/collab" },
        ]),
      );

export default defineConfig({
  plugins: [react(), vellumServiceWorker()],
  ...(proxy === undefined ? {} : { server: { proxy } }),
  build: {
    sourcemap: true,
    // The editor chunk (Tiptap, ProseMirror, Yjs) is about 560 kB; it loads lazily.
    chunkSizeWarningLimit: 600,
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
