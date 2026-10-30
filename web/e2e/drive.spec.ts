// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { signedIn } from "./support/server-session";

// Feature tests: once per browser, at laptop size. The UI suite covers sizes.
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

test("the page tree comes from the server's drive and changes it", async ({ page }) => {
  const drive = await signedIn(page);
  await page.goto("/");
  const tree = page.getByRole("tree");
  await expect(tree.getByRole("treeitem", { name: "Plans" })).toBeVisible();
  await expect(tree.getByRole("treeitem", { name: "Roadmap" })).toBeVisible();
  await expect(page.getByRole("heading", { level: 2, name: "My Drive" })).toBeVisible();
  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);

  // A new page gets the server's ID, then a title.
  await page.getByRole("button", { name: "Add a page to My Drive" }).click();
  await expect(page).toHaveURL(/\/p\/01M6400000PAGE00NEW0000001$/);
  const title = page.getByRole("textbox", { name: "Page title" });
  await title.fill("Launch notes");
  await title.press("Enter");
  await expect(tree.getByRole("treeitem", { name: "Launch notes" })).toBeVisible();
  await page.getByRole("button", { name: "Favorite" }).click();
  await expect(page.getByRole("button", { name: "Favorite" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect.poll(() => drive.starred.has("01M6400000PAGE00NEW0000001")).toBe(true);

  // A folder lists its pages.
  await tree.getByRole("treeitem", { name: "Plans" }).click();
  await expect(page.getByText("A folder. The pages in it:")).toBeVisible();
  await expect(page.getByRole("main").getByRole("link", { name: /Roadmap/ })).toBeVisible();

  // Search asks the server.
  await page.goto("/search?q=road");
  await expect(page.getByRole("status")).toHaveText("One page found");

  // After a reload everything comes back from the server.
  await page.goto("/favorites");
  await expect(page.getByRole("main").getByRole("link", { name: /Launch notes/ })).toBeVisible();
  expect(drive.changes).toEqual([
    "POST /spaces/01M6400000SPACE0MYDRIVE001/pages",
    "PATCH /items/01M6400000PAGE00NEW0000001",
    "PUT /items/01M6400000PAGE00NEW0000001/star",
  ]);
});

test("a change the server refuses is undone and says why", async ({ page }) => {
  const drive = await signedIn(page);
  drive.refuseNext("/trash", {
    status: 403,
    code: "permission_denied",
    message: "you may only view this folder",
    request_id: "01M6409MCMEH1TRKXZ1VVZ4C7H",
  });
  await page.goto("/");
  const tree = page.getByRole("tree");
  await tree.getByRole("button", { name: "Actions for Plans" }).click();
  await page.getByRole("menuitem", { name: "Move to trash" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "you may only view this folder (request 01M6409MCMEH1TRKXZ1VVZ4C7H)",
  );
  await expect(tree.getByRole("treeitem", { name: "Plans" })).toBeVisible();
  await page.getByRole("button", { name: "Dismiss" }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);

  // The next try goes through.
  await tree.getByRole("button", { name: "Actions for Plans" }).click();
  await page.getByRole("menuitem", { name: "Move to trash" }).click();
  await expect(tree.getByRole("treeitem", { name: "Plans" })).toHaveCount(0);
  expect(drive.items.map((item) => item.title)).toEqual(["Roadmap"]);
});
