// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { vellumServiceWorker } from "./src/platform/pwa/vite-plugin";

export default defineConfig({
  plugins: [react(), vellumServiceWorker()],
  build: {
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/main.tsx", "src/platform/pwa/sw.ts"],
      thresholds: { lines: 85, functions: 85, branches: 85, statements: 85 },
    },
  },
});
