// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { expect, test } from "@playwright/test";

test("the installed shell starts offline", async ({ page, context }) => {
  await page.goto("/");
  await page.evaluate(async () => {
    await navigator.serviceWorker.ready;
  });
  await context.setOffline(true);
  await page.goto("/docs/anything");
  await expect(page.getByRole("heading", { level: 1, name: "Vellum" })).toBeVisible();
  await context.setOffline(false);
});

test("the manifest makes the app installable", async ({ page, request }) => {
  await page.goto("/");
  const href = await page.locator('link[rel="manifest"]').getAttribute("href");
  expect(href).toBe("/manifest.webmanifest");
  const response = await request.get(href ?? "");
  const manifest = (await response.json()) as {
    display: string;
    icons: { purpose: string }[];
  };
  expect(manifest.display).toBe("standalone");
  expect(manifest.icons.some((icon) => icon.purpose === "maskable")).toBe(true);
});
