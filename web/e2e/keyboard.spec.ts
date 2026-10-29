// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

// WebKit sometimes navigated during the axe scan while the offline worker
// installed; blocking it removed that. The PWA has its own tests.
test.use({ serviceWorkers: "block" });

test("the command menu opens from the keyboard and passes WCAG 2.2 AA", async ({ page }) => {
  // Reduced motion turns off the 80 ms pop-in, so axe never measures
  // half-faded text and no animation is left to wait on.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+k");
  const search = page.getByRole("combobox", { name: "Search pages and commands" });
  await expect(search).toBeFocused();
  await search.fill("short");
  await expect(page.getByRole("option", { name: /Show all shortcuts/ })).toBeVisible();
  const results = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(results.violations).toEqual([]);

  // Pages whose titles match come first; pick the command itself.
  await page.getByRole("option", { name: /Show all shortcuts/ }).click();
  const sheet = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(sheet).toBeVisible();
  const sheetResults = await new AxeBuilder({ page }).withTags(WCAG).analyze();
  expect(sheetResults.violations).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
});
