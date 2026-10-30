// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { fakeDrive } from "./support/fake-drive";

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

const USER = {
  id: "01M6409GN2EYWS8Z7NFQQ3JNWZ",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: true,
};

/**
 * Stands in for a Vellum server: the status starts as given and moves on
 * when someone sets up or signs in, as `vellum serve` does.
 */
async function fakeServer(page: Page, start: "new" | "signed-out") {
  let status: Record<string, unknown> =
    start === "new"
      ? { needsSetup: true, credential: "anonymous", scopes: [] }
      : { needsSetup: false, credential: "anonymous", scopes: [] };
  await page.route("**/api/v1/auth/status", (route) => route.fulfill({ json: status }));
  await page.route("**/api/csrf", (route) =>
    route.fulfill({ json: { token: "t", header: "x-csrf-token" } }),
  );
  await page.route("**/api/v1/setup", (route) => route.fulfill({ json: USER }));
  await page.route("**/auth/sign-in", async (route) => {
    const body = route.request().postDataJSON() as { password: string };
    if (body.password !== "a long enough password") {
      await route.fulfill({
        status: 401,
        json: {
          code: "unauthenticated",
          message: "the email or password is wrong",
          retryable: false,
          request_id: "01M6409MCMEH1TRKXZ1VVZ4C7H",
        },
      });
      return;
    }
    status = { needsSetup: false, credential: "session", scopes: ["read"], user: USER };
    await route.fulfill({ json: { user: USER, sessionId: "s1" } });
  });
  await page.route("**/api/v1/collections", (route) =>
    route.fulfill({ json: { collections: [] } }),
  );
  await fakeDrive(page, USER.id);
}

async function expectNoViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

test("a new workspace creates its first administrator", async ({ page }) => {
  await fakeServer(page, "new");
  await page.goto("/data");
  await expect(
    page.getByRole("heading", { level: 1, name: "Set up your workspace" }),
  ).toBeVisible();
  await expectNoViolations(page);
  await page.getByLabel("Your name").fill("Priya Raman");
  await page.getByLabel("Email").fill("priya@northstar.dev");
  await page.getByLabel("Password").fill("a long enough password");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Good to see you, Priya" }),
  ).toBeVisible();
});

test("signing in returns to the page asked for, and says why it failed", async ({ page }) => {
  await fakeServer(page, "signed-out");
  await page.goto("/data");
  await expect(page).toHaveURL(/\/sign-in\?next=%2Fdata$/);
  await expectNoViolations(page);
  await page.getByLabel("Email").fill("priya@northstar.dev");
  await page.getByLabel("Password").fill("not the password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("alert")).toHaveText(
    "the email or password is wrong (request 01M6409MCMEH1TRKXZ1VVZ4C7H)",
  );
  await page.getByLabel("Password").fill("a long enough password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/data$/);
  await expect(page.getByText("There are no collections yet.")).toBeVisible();
});
