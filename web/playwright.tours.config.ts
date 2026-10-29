// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig } from "@playwright/test";

import { APP_PORT } from "./e2e/ui/targets";

/**
 * The help videos (ADR 0024, D258): `pnpm run tours` records each tour in
 * e2e/tours at 1280×720 against the Northstar demo build. Each test's
 * folder in test-results/tours holds video.webm and captions.vtt.
 */
// A sandbox with a preinstalled Chromium of another revision can point at it.
const chromiumPath = process.env["UI_CHROMIUM_PATH"];

export default defineConfig({
  testDir: "e2e/tours",
  testMatch: "**/*.tour.ts",
  outputDir: "test-results/tours",
  timeout: 120_000,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${String(APP_PORT)}`,
    browserName: "chromium",
    viewport: { width: 1280, height: 720 },
    colorScheme: "light",
    video: { mode: "on", size: { width: 1280, height: 720 } },
    ...(chromiumPath === undefined ? {} : { launchOptions: { executablePath: chromiumPath } }),
  },
  webServer: {
    command: "pnpm run build && pnpm run preview",
    url: `http://127.0.0.1:${String(APP_PORT)}`,
    reuseExistingServer: !process.env["CI"],
  },
});
