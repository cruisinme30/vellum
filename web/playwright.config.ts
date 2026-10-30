// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { defineConfig, type Project } from "@playwright/test";

import { APP_PORT, SITE_DIR, SITE_PORT } from "./e2e/ui/targets";

/**
 * The browser matrix (ADR 0031, D124): every browser at every viewport.
 * UI_BROWSERS and UI_VIEWPORTS (comma-separated) narrow it; the fast
 * suite runs Chromium at phone and laptop, the full suite everything.
 */
const BROWSERS = ["chromium", "firefox", "webkit"] as const;
const VIEWPORTS = {
  phone: { viewport: { width: 390, height: 844 }, hasTouch: true },
  tablet: { viewport: { width: 820, height: 1180 }, hasTouch: true },
  laptop: { viewport: { width: 1366, height: 768 }, hasTouch: false },
  wide: { viewport: { width: 2560, height: 1440 }, hasTouch: false },
} as const;

const pick = (
  variable: string,
  all: readonly string[],
  fallback: readonly string[],
): string[] => {
  const wanted =
    process.env[variable]?.split(",").map((value) => value.trim()) ?? fallback;
  return all.filter((value) => wanted.includes(value));
};
const browsers = pick("UI_BROWSERS", BROWSERS, ["chromium"]);
const viewports = pick(
  "UI_VIEWPORTS",
  Object.keys(VIEWPORTS),
  Object.keys(VIEWPORTS),
);
// A sandbox with a preinstalled Chromium of another revision can point at it.
const chromiumPath = process.env["UI_CHROMIUM_PATH"];

const projects: Project[] = browsers.flatMap((browser) =>
  viewports.map((viewport) => ({
    name: `${browser}-${viewport}`,
    use: {
      browserName: browser as (typeof BROWSERS)[number],
      ...VIEWPORTS[viewport as keyof typeof VIEWPORTS],
      // Phone emulation (isMobile) only exists in Chromium and WebKit.
      isMobile: viewport === "phone" && browser !== "firefox",
      ...(browser === "chromium" && chromiumPath !== undefined
        ? { launchOptions: { executablePath: chromiumPath } }
        : {}),
    },
  })),
);

export default defineConfig({
  testDir: "e2e",
  forbidOnly: Boolean(process.env["CI"]),
  retries: 0,
  fullyParallel: true,
  // A book's print page holds the whole book; auditing it in Firefox with
  // every text lengthened takes longer than the default 30 s.
  timeout: 90_000,
  reporter: process.env["CI"]
    ? [
        ["github"],
        ["html", { open: "never" }],
        ["json", { outputFile: "test-results/results.json" }],
      ]
    : [["list"], ["json", { outputFile: "test-results/results.json" }]],
  // Visual baselines are shared by every machine that runs the Playwright
  // container (D125), so the path names the project, not the OS.
  snapshotPathTemplate:
    "{testDir}/__screenshots__/{projectName}/{testFileName}/{arg}{ext}",
  expect: {
    // The brief's pixel threshold for golden fixtures: 0.1 % of the image.
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.001,
      animations: "disabled",
      caret: "hide",
    },
  },
  use: {
    baseURL: `http://127.0.0.1:${String(APP_PORT)}`,
    trace: "retain-on-failure",
  },
  projects,
  webServer: [
    {
      command: "pnpm run build && pnpm run preview",
      url: `http://127.0.0.1:${String(APP_PORT)}`,
      reuseExistingServer: !process.env["CI"],
    },
    {
      command: `node e2e/support/serve-static.ts ${JSON.stringify(SITE_DIR)} ${String(SITE_PORT)}`,
      url: `http://127.0.0.1:${String(SITE_PORT)}/index.html`,
      reuseExistingServer: !process.env["CI"],
    },
  ],
});
