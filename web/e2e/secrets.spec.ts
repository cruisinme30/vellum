// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { fakeCollab } from "./support/fake-collab";
import { signedIn } from "./support/server-session";

// The secret-warning banner (R-SE-6, D296, #527): the editor asks the
// server's scanner about the page and names each likely secret's kind and
// line, with Remove and "It's not a secret". Feature tests: once per
// browser, at laptop size.
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

const PAGE = "01M6400000PAGE00ROADMAP001";
/** A GitHub token shaped string, built so no source file holds one. */
const TOKEN = ["ghp_", "Zq9Xw8Vu7Ts6Rq5Po4Nm3Lk2Ji1Hg0FeDcBa"].join("");
const PATTERN = /ghp_[A-Za-z0-9]{36}/u;

/**
 * Stands in for `POST /api/v1/security/secrets/scan` under `policy`: one
 * finding per line holding a GitHub token, as the server reports it.
 */
async function fakeScanner(page: Page, policy: "warn" | "block") {
  const scanned: string[] = [];
  await page.route("**/api/v1/security/secrets/scan", (route) => {
    const body = route.request().postDataJSON() as { text?: unknown };
    const text = typeof body.text === "string" ? body.text : "";
    scanned.push(text);
    const findings = text.split("\n").flatMap((line, index) => {
      const match = PATTERN.exec(line);
      if (match === null) return [];
      return [
        {
          rule: "github-token",
          line: index + 1,
          column: match.index + 1,
          length: match[0].length,
          redacted: `${match[0].slice(0, 4)}…(${String(match[0].length)})`,
        },
      ];
    });
    return route.fulfill({ json: { policy, findings } });
  });
  return scanned;
}

test("a likely secret shows a banner that names it, and Remove takes it out", async ({
  page,
}) => {
  await signedIn(page);
  const room = await fakeCollab(page, {
    paragraph: `The deploy bot pushes with ${TOKEN}.`,
  });
  const scanned = await fakeScanner(page, "warn");
  await page.goto(`/p/${PAGE}`);

  const banner = page
    .getByRole("alert")
    .filter({ hasText: "looks like it holds a secret" });
  await expect(banner).toBeVisible();
  await expect(banner).toContainText(
    /GitHub token on line \D?1\D? \(ghp_…\(40\)\)/u,
  );
  // Only the redacted preview is ever shown, never the token itself.
  await expect(banner).not.toContainText(TOKEN);
  expect(scanned.some((text) => text.includes(TOKEN))).toBe(true);
  const axe = await new AxeBuilder({ page })
    .include(".vl-secrets-banner")
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);

  await banner.getByRole("button", { name: "Remove" }).click();
  await expect.poll(() => room.text()).toBe("The deploy bot pushes with .");
  await expect(banner).toBeHidden();
});

test("It's not a secret hides the warning for that finding", async ({
  page,
}) => {
  await signedIn(page);
  const room = await fakeCollab(page, { paragraph: `Example key: ${TOKEN}` });
  await fakeScanner(page, "warn");
  await page.goto(`/p/${PAGE}`);

  const banner = page
    .getByRole("alert")
    .filter({ hasText: "looks like it holds a secret" });
  await expect(banner).toBeVisible();
  await banner.getByRole("button", { name: "It's not a secret" }).click();
  await expect(banner).toBeHidden();
  // The page keeps the text.
  expect(room.text()).toBe(`Example key: ${TOKEN}`);
});

test("under the block policy the banner says the page can't be saved", async ({
  page,
}) => {
  await signedIn(page);
  await fakeCollab(page, { paragraph: `Token: ${TOKEN}` });
  await fakeScanner(page, "block");
  await page.goto(`/p/${PAGE}`);

  const banner = page
    .getByRole("alert")
    .filter({ hasText: "can't be saved until you remove it" });
  await expect(banner).toBeVisible();
  await expect(banner).toHaveAttribute("data-policy", "block");
  await expect(banner.getByRole("button", { name: "Remove" })).toBeVisible();
  // A blocked secret cannot be waved through from the editor.
  await expect(
    banner.getByRole("button", { name: "It's not a secret" }),
  ).toHaveCount(0);
});

test("a page with no likely secret shows no banner", async ({ page }) => {
  await signedIn(page);
  await fakeCollab(page, { paragraph: "Freeze on Fridays." });
  const scanned = await fakeScanner(page, "warn");
  await page.goto(`/p/${PAGE}`);
  await expect(page.getByRole("textbox", { name: "Page body" })).toContainText(
    "Freeze on Fridays.",
  );
  await expect.poll(() => scanned.length).toBeGreaterThan(0);
  await expect(page.locator(".vl-secrets-banner")).toHaveCount(0);
});
