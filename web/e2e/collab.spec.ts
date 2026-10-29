// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { expect, test } from "@playwright/test";

import { fakeCollab, fakeRoom } from "./support/fake-collab";
import { SAM, signedIn } from "./support/server-session";

// Feature tests: once per browser, at laptop size. The UI suite covers sizes.
test.use({ viewport: { width: 1366, height: 768 }, hasTouch: false, isMobile: false });
// eslint-disable-next-line no-empty-pattern -- Playwright needs the fixtures object
test.beforeEach(({}, testInfo) => {
  test.skip(!testInfo.project.name.endsWith("-laptop"), "runs at laptop size only");
});

const SPACE = "01M6400000SPACE0MYDRIVE001";
const PAGE = "01M6400000PAGE00ROADMAP001";

test("a page body loads from its live room and edits go back to it", async ({ page }) => {
  await signedIn(page);
  const room = await fakeCollab(page, { paragraph: "Ship the beta in May." });
  await page.goto(`/p/${PAGE}`);
  const body = page.getByRole("textbox", { name: "Page body" });
  await expect(body).toContainText("Ship the beta in May.");
  expect(room.urls).toEqual([expect.stringMatching(new RegExp(`/collab/${SPACE}/${PAGE}$`))]);

  await expect(body).toHaveAttribute("contenteditable", "true");
  await body.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" Then GA.");
  await expect.poll(() => room.text()).toBe("Ship the beta in May. Then GA.");
});

test("a viewer sees the page read-only, with a notice", async ({ page }) => {
  await signedIn(page);
  await fakeCollab(page, { canWrite: false, paragraph: "Read me." });
  await page.goto(`/p/${PAGE}`);
  const body = page.getByRole("textbox", { name: "Page body" });
  await expect(body).toContainText("Read me.");
  await expect(page.getByText("You can view this page but not edit it.")).toBeVisible();
  await expect(body).toHaveAttribute("contenteditable", "false");
});

test("two browsers edit one page and see each other (#98)", async ({ browser, page }) => {
  const room = fakeRoom({ paragraph: "Ship the beta in May." });
  await signedIn(page);
  await room.join(page);
  const other = await browser.newContext({ viewport: { width: 1366, height: 768 } });
  try {
    const samPage = await other.newPage();
    await signedIn(samPage, SAM);
    await room.join(samPage, { user: SAM.id });

    await page.goto(`/p/${PAGE}`);
    await samPage.goto(`/p/${PAGE}`);
    const priyaBody = page.getByRole("textbox", { name: "Page body" });
    const samBody = samPage.getByRole("textbox", { name: "Page body" });
    await expect(priyaBody).toContainText("Ship the beta in May.");
    await expect(samBody).toContainText("Ship the beta in May.");

    // Each sees the other in the presence bar.
    await expect(page.getByRole("listitem", { name: "Sam Okafor is editing" })).toBeVisible();
    await expect(samPage.getByRole("listitem", { name: "Priya Raman is editing" })).toBeVisible();

    // An edit in one browser shows in the other, with the writer's caret.
    await samBody.click();
    await samPage.keyboard.press("End");
    await samPage.keyboard.type(" Then GA.");
    await expect(priyaBody).toContainText("Ship the beta in May. Then GA.");
    await expect(page.locator(".collaboration-carets__label")).toHaveText("Sam Okafor");
    await expect.poll(() => room.text()).toBe("Ship the beta in May. Then GA.");

    // Sam leaves the page; Priya's bar empties.
    await samPage.goto("/");
    await expect(page.getByRole("listitem", { name: "Sam Okafor is editing" })).toHaveCount(0);
  } finally {
    await other.close();
  }
});
