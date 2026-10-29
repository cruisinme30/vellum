// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      // The entry points only wire Electron modules to the tested logic.
      exclude: [
        "src/**/*.test.ts",
        "src/main/index.ts",
        "src/main/electron-adapters.ts",
        "src/preload/preload.cts",
      ],
      thresholds: { lines: 85, functions: 85, branches: 85, statements: 85 },
    },
  },
});
