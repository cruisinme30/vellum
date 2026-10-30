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
  test.skip(
    !testInfo.project.name.endsWith("-laptop"),
    "runs at laptop size only",
  );
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
  await page.route("**/api/v1/auth/status", (route) =>
    route.fulfill({ json: status }),
  );
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
    status = {
      needsSetup: false,
      credential: "session",
      scopes: ["read"],
      user: USER,
    };
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

test("signing in returns to the page asked for, and says why it failed", async ({
  page,
}) => {
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

test("a second step asks for the authenticator code", async ({ page }) => {
  await fakeServer(page, "signed-out");
  let signedIn = false;
  await page.route("**/api/v1/auth/status", (route) =>
    route.fulfill({
      json: signedIn
        ? {
            needsSetup: false,
            credential: "session",
            scopes: ["read"],
            user: USER,
          }
        : { needsSetup: false, credential: "anonymous", scopes: [] },
    }),
  );
  await page.route("**/auth/sign-in", (route) =>
    route.fulfill({
      json: {
        secondFactor: { ticket: "vmt_e2e", methods: ["totp", "recovery"] },
      },
    }),
  );
  await page.route("**/auth/sign-in/verify", async (route) => {
    const body = route.request().postDataJSON() as { code?: string };
    if (body.code !== "123456") {
      await route.fulfill({
        status: 401,
        json: {
          code: "unauthenticated",
          message: "the code is wrong",
          retryable: false,
        },
      });
      return;
    }
    signedIn = true;
    await route.fulfill({ json: { user: USER, sessionId: "s1" } });
  });
  await page.goto("/sign-in?next=%2Fdata");
  await page.getByLabel("Email").fill("priya@northstar.dev");
  await page.getByLabel("Password").fill("a long enough password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(
    page.getByRole("heading", { name: "Two-step verification" }),
  ).toBeVisible();
  await expectNoViolations(page);
  await page.getByLabel("Code from your authenticator app").fill("000000");
  await page.getByRole("button", { name: "Verify" }).click();
  await expect(page.getByRole("alert")).toContainText("the code is wrong");
  await page.getByLabel("Code from your authenticator app").fill("123456");
  await page.getByRole("button", { name: "Verify" }).click();
  await expect(page).toHaveURL(/\/data$/);
});

test("single sign-on goes to the SAML provider and back", async ({ page }) => {
  await fakeServer(page, "signed-out");
  let signedIn = false;
  const entra = {
    id: "entra",
    name: "Entra ID",
    startUrl: "/auth/sso/start?provider=entra",
  };
  await page.route("**/api/v1/auth/status", (route) =>
    route.fulfill({
      json: signedIn
        ? {
            needsSetup: false,
            credential: "session",
            scopes: ["read"],
            user: USER,
          }
        : {
            needsSetup: false,
            credential: "anonymous",
            scopes: [],
            sso: entra,
            providers: [entra],
          },
    }),
  );
  await page.goto("/sign-in?next=%2Fdata");
  const app = new URL(page.url()).origin;
  // The server sends the browser to the provider with a request. The real
  // server answers 302; here a page moves on instead, because Chromium does
  // not hand the hop after a faked 302 to the routes below, so the made-up
  // provider host would be looked up for real and fail.
  await page.route("**/auth/sso/start?**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: `<!doctype html><meta http-equiv="refresh" content="0;url=https://idp.northstar.test/sso?SAMLRequest=x&amp;RelayState=r">`,
    }),
  );
  // ...the provider signs the person in and posts the response back...
  await page.route("https://idp.northstar.test/**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: `<!doctype html><html lang="en"><title>Northstar IdP</title><main><h1>Northstar sign-in</h1>
        <form method="post" action="${app}/saml/acs">
          <input type="hidden" name="SAMLResponse" value="PHNhbWxwOlJlc3BvbnNlLz4=">
          <input type="hidden" name="RelayState" value="r">
          <button>Continue</button>
        </form></main></html>`,
    }),
  );
  // ...and the server checks it, opens a session and goes on. The real
  // server answers 303; Playwright's WebKit refuses to fulfil a faked
  // redirect, so this page moves on the same way the start route does.
  await page.route("**/saml/acs", async (route) => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().postData()).toContain("RelayState=r");
    signedIn = true;
    await route.fulfill({
      contentType: "text/html",
      body: `<!doctype html><meta http-equiv="refresh" content="0;url=${app}/data">`,
    });
  });
  await page.getByRole("link", { name: "Sign in with Entra ID" }).click();
  await expect(
    page.getByRole("heading", { name: "Northstar sign-in" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page).toHaveURL(/\/data$/);
  await expect(page.getByText("There are no collections yet.")).toBeVisible();
});
