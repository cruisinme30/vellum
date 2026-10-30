// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { readFile } from "node:fs/promises";

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

// Feature tests for Settings → Appearance (R-TH-1 to R-TH-5): once per
// browser at laptop size. The UI suite (e2e/ui) covers the page at every
// size and display mode.
test.use({ viewport: { width: 1366, height: 768 }, hasTouch: false, isMobile: false });
// eslint-disable-next-line no-empty-pattern -- Playwright needs the fixtures object
test.beforeEach(({}, testInfo) => {
  test.skip(!testInfo.project.name.endsWith("-laptop"), "runs at laptop size only");
});

const PAGE = "/settings/appearance";
const THEMES = ["Light", "Dark", "High contrast", "Sepia", "Dim"] as const;

async function expectNoViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

const html = (page: Page, name: string) =>
  page.evaluate((attribute) => document.documentElement.getAttribute(attribute), name);

const editor = (page: Page) => page.getByRole("region", { name: "Custom theme" });

test("every built-in theme passes axe on the appearance page", async ({ page }) => {
  await page.goto(PAGE);
  const themes = page.getByRole("main").getByRole("group", { name: "Theme" });
  for (const theme of THEMES) {
    await themes.getByRole("radio", { name: theme, exact: true }).check();
    await expect.poll(() => html(page, "data-theme")).toBe(theme.toLowerCase().replace(" ", "-"));
    await expectNoViolations(page);
  }
});

test("reading settings apply at once and survive a reload", async ({ page }) => {
  await page.goto(PAGE);
  await page.getByRole("radio", { name: "175 %" }).check();
  await page.getByRole("radio", { name: "OpenDyslexic" }).check();
  await page.getByRole("radio", { name: "Spacious" }).check();
  await page.getByRole("radio", { name: "Reduce motion" }).check();
  await page.reload();
  expect(await html(page, "data-font")).toBe("dyslexic");
  expect(await html(page, "data-motion")).toBe("reduce");
  expect(await html(page, "data-density")).toBe("spacious");
  const size = await page.evaluate(() => getComputedStyle(document.documentElement).fontSize);
  expect(Number.parseFloat(size)).toBeCloseTo(28, 0);
  // The dyslexia font is fetched only when chosen, and is used once loaded.
  await expect
    .poll(() => page.evaluate(() => document.fonts.check('16px "OpenDyslexic"')))
    .toBe(true);
  const family = await page
    .getByRole("heading", { level: 1 })
    .evaluate((element) => getComputedStyle(element).fontFamily);
  expect(family).toContain("OpenDyslexic");
  // Reduced motion stills transitions everywhere.
  const transition = await page
    .getByRole("button", { name: "Use this theme" })
    .evaluate((element) => getComputedStyle(element).transitionDuration);
  expect(transition.split(",").every((value) => Number.parseFloat(value) < 0.001)).toBe(true);
  // Large text still fits the window: nothing scrolls sideways.
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
  await expectNoViolations(page);
});

test("allow motion overrides the system's reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(PAGE);
  await page.getByRole("radio", { name: "Allow motion" }).check();
  const motion = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--vl-motion").trim(),
  );
  // Minified to ".2s"; reduced motion would make it 0.
  expect(Number.parseFloat(motion)).toBeGreaterThan(0);
});

test("the theme editor checks contrast, exports and re-imports a theme", async ({ page }, info) => {
  await page.goto(PAGE);
  const custom = editor(page);
  await custom.getByLabel("Theme name").fill("Northstar ink");
  await custom.locator("summary", { hasText: "Colours" }).click();
  // Body text almost the page colour: the theme cannot be used.
  await custom.getByLabel("on-surface", { exact: true }).fill("#f2f2f2");
  await expect(custom.getByRole("status").first()).toContainText("too little contrast");
  await expect(custom.getByRole("button", { name: "Use this theme" })).toBeDisabled();
  await custom.getByRole("button", { name: "Reset colours" }).click();
  await expect(custom.getByRole("status").first()).toContainText("pass");

  const downloading = page.waitForEvent("download");
  await custom.getByRole("button", { name: "Export JSON" }).click();
  const download = await downloading;
  expect(download.suggestedFilename()).toBe("northstar-ink.vellum-theme.json");
  const saved = info.outputPath("theme.json");
  await download.saveAs(saved);
  const text = await readFile(saved, "utf8");
  expect(JSON.parse(text)).toMatchObject({ format: "vellum.theme/1", name: "Northstar ink" });

  await custom.getByLabel("Theme name").fill("Something else");
  await custom.getByLabel("Import JSON").setInputFiles(saved);
  await expect(custom.getByText(/Imported "Northstar ink"/)).toBeVisible();
  await custom.getByRole("button", { name: "Use this theme" }).click();
  expect(await html(page, "data-custom-theme")).toBe("Northstar ink");
  await expectNoViolations(page);

  const bad = JSON.stringify({
    format: "vellum.theme/1",
    name: "Fog",
    base: "light",
    seed: "#225588",
    tokens: { "on-surface": "#eeeeee" },
  });
  await custom.getByLabel("Import JSON").setInputFiles({
    name: "fog.json",
    mimeType: "application/json",
    buffer: Buffer.from(bad),
  });
  await expect(custom.getByRole("alert")).toContainText("not imported");
});

test("the quick theme menu links to every appearance setting", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Theme" }).first().click();
  await page.getByRole("link", { name: "All appearance settings" }).click();
  await expect(page).toHaveURL(/\/settings\/appearance$/);
  await expect(page.getByRole("heading", { level: 1, name: "Appearance" })).toBeVisible();
});
