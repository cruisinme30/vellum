// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

// These are feature tests: they run once per browser, at laptop size. The UI
// suite (e2e/ui) covers every screen size; the phone cases below set their own.
// The offline worker is blocked: once it controls the page, WebKit and
// Firefox send the app's API calls past the routes that fake the server.
// The PWA has its own tests.
test.use({
  viewport: { width: 1366, height: 768 },
  hasTouch: false,
  isMobile: false,
  serviceWorkers: "block",
});
// eslint-disable-next-line no-empty-pattern -- Playwright needs the fixtures object
test.beforeEach(({}, testInfo) => {
  test.skip(!testInfo.project.name.endsWith("-laptop"), "runs at laptop size only");
});

const ISSUES = "/data/01JB3KD0COLL0ISSUES0000001";

async function expectNoViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

for (const [name, view, ready] of [
  ["collections index", "", "link"],
  ["table view", "?view=All+issues", "grid"],
  ["board view", "?view=Board", "region"],
  ["gallery view", "?view=Gallery", "list"],
  ["calendar view", "?view=Calendar", "table"],
  ["timeline view", "?view=Timeline", "list"],
  ["list view", "?view=Open", "region"],
] as const) {
  test(`the ${name} has no WCAG 2.2 AA violations`, async ({ page }) => {
    await page.goto(view === "" ? "/data" : `${ISSUES}${view}`);
    await expect(page.getByRole(ready).first()).toBeVisible();
    await expectNoViolations(page);
  });
}

test("the row panel has no WCAG 2.2 AA violations", async ({ page }) => {
  await page.goto(`${ISSUES}?view=Gallery`);
  await page.getByRole("button", { name: /^Token bucket per tenant/ }).click();
  await expect(page.getByRole("textbox", { name: "Issue" })).toBeVisible();
  await expectNoViolations(page);
});

test("a table cell is edited from the keyboard", async ({ page }) => {
  await page.goto(ISSUES);
  const grid = page.getByRole("grid", { name: "All issues" });
  await expect(grid).toBeVisible();
  // The grid has one tabbable cell, the first title.
  const first = grid.getByRole("gridcell").first();
  await first.focus();
  await expect(first).toBeFocused();
  await page.keyboard.press("ArrowRight");
  const status = grid.getByRole("row").nth(1).getByRole("gridcell").nth(1);
  await expect(status).toBeFocused();
  await page.keyboard.press("Enter");
  const listbox = status.getByRole("listbox");
  await expect(listbox).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await expect(status).toHaveText("In review");
  await expect(status).toBeFocused();

  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("F2");
  const input = grid.getByRole("textbox", { name: "Issue of Trace sampling knobs" });
  await expect(input).toBeFocused();
  await input.fill("Trace sampling controls");
  await page.keyboard.press("Enter");
  await expect(grid.getByRole("gridcell", { name: /Trace sampling controls/ })).toBeFocused();

  await page.keyboard.press("F2");
  await page.keyboard.type(" (draft)");
  await page.keyboard.press("Escape");
  await expect(grid.getByRole("gridcell", { name: /Trace sampling controls$/ })).toBeVisible();
});

test("a board card is moved from the keyboard", async ({ page }) => {
  await page.goto(`${ISSUES}?view=Board`);
  const review = page.getByRole("region", { name: /^In review/ });
  const done = page.getByRole("region", { name: /^Done/ });
  const card = page.getByRole("button", { name: /^Gateway returns 502 on cold start/ });
  await expect(review).toContainText("Gateway returns 502 on cold start");

  await card.focus();
  await page.keyboard.press("Alt+ArrowRight");
  await expect(done).toContainText("Gateway returns 502 on cold start");
  await expect(card).toBeFocused();
  await expect(page.getByRole("status")).toHaveText(
    "Moved Gateway returns 502 on cold start to Done.",
  );

  // The Move to menu is the other keyboard path.
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Move Gateway returns 502 on cold start" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menuitem", { name: "Move to Backlog" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("region", { name: /^Backlog/ })).toContainText(
    "Gateway returns 502 on cold start",
  );
});

test("a board card is moved by dragging", async ({ page }) => {
  await page.goto(`${ISSUES}?view=Board`);
  const card = page.getByRole("button", { name: /^Token bucket per tenant/ });
  await card.dragTo(page.getByRole("region", { name: /^In review/ }));
  await expect(page.getByRole("region", { name: /^In review/ })).toContainText(
    "Token bucket per tenant",
  );
});
