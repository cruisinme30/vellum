// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { signedIn } from "./support/server-session";

// Feature tests: once per browser, at laptop size. The UI suite covers sizes.
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

const BOOK_ID = "01M6400000BOOK000FIELDGD01";

test("New → Book makes a book in My Drive and opens it (#566)", async ({ page }) => {
  await signedIn(page);
  const made: unknown[] = [];
  await page.route("**/api/v1/books", async (route) => {
    if (route.request().method() === "POST") {
      made.push(route.request().postDataJSON());
      return route.fulfill({
        json: { id: BOOK_ID, title: "Field guide", path: "drives/users/priya/Field guide.book" },
      });
    }
    return route.fulfill({ json: { books: [] } });
  });
  await page.route(`**/api/v1/books/${BOOK_ID}`, (route) =>
    route.fulfill({
      json: {
        id: BOOK_ID,
        title: "Field guide",
        max_depth: 3,
        numbering: "decimal",
        role: "owner",
        toc: [],
      },
    }),
  );

  await page.goto("/");
  await expect(page.getByRole("treeitem", { name: "Plans" })).toBeVisible();
  await page.getByRole("button", { name: "New" }).click();
  await expect(page.getByRole("menu", { name: "Make something new" })).toBeVisible();
  await page.getByRole("menuitem", { name: "Book" }).click();

  await expect(page).toHaveURL(/\/books\?new=book$/);
  const form = page.getByRole("form", { name: "New book" });
  await expect(form.getByRole("combobox", { name: "Drive" })).toHaveValue("drives/users/priya");
  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);
  await form.getByRole("textbox", { name: "Title" }).fill("Field guide");
  await form.getByRole("button", { name: "Create book" }).click();

  await expect(page).toHaveURL(new RegExp(`/books/${BOOK_ID}$`));
  await expect(page.getByRole("heading", { level: 1, name: "Field guide" })).toBeVisible();
  expect(made).toEqual([{ title: "Field guide", folder: "drives/users/priya" }]);
});
