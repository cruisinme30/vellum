// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Conditional skips say why in their message: a browser or setup lacks the
// feature.
/* eslint-disable sonarjs/no-skipped-tests */

import type { Page } from "@playwright/test";

import { expect, test } from "./fixtures";
import { CLS_LIMIT, layoutShift, settle, watchLayoutShift } from "./harness";
import { fakeServer } from "./server";
import { TARGETS } from "./targets";

/**
 * Loading states (ADR 0031, D123): a slow network, no JavaScript, a
 * script that fails to load, and going offline. The page must never sit
 * blank, and must not jump as it fills in.
 */

/** "Slow 4G": 150 ms round trip, 1.6 Mbit/s down. */
const SLOW = {
  latency: 150,
  downloadThroughput: 200_000,
  uploadThroughput: 90_000,
};
/**
 * Users see something within this time on the slow network (Core Web Vitals
 * "good" LCP).
 */
const FIRST_TEXT_MS = 2500;

// innerText, not textContent: only text a reader can see counts.
const hasText = (page: Page): Promise<boolean> =>
  page.evaluate(
    () =>
      // eslint-disable-next-line unicorn/prefer-dom-node-text-content
      (document.body as HTMLElement | null)?.innerText.trim() !== "",
  );

// Sweep pages share the template of the pages checked here (D165).
for (const target of TARGETS.filter((candidate) => !candidate.sweep)) {
  test.describe(`${target.name} loading`, () => {
    test("shows content quickly and without jumping on a slow network", async ({
      page,
      browserName,
    }) => {
      test.skip(browserName !== "chromium", "network throttling needs CDP");
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Network.enable");
      await cdp.send("Network.emulateNetworkConditions", {
        offline: false,
        ...SLOW,
      });
      await watchLayoutShift(page);
      await fakeServer(page, target.server);
      const started = Date.now();
      await page.goto(target.url, { waitUntil: "commit" });
      await expect
        .poll(() => hasText(page), { timeout: 30_000, intervals: [50] })
        .toBe(true);
      const firstText = Date.now() - started;
      // On a throttled link the whole of a long page can take minutes; the
      // check is about what readers see first, so stop waiting after 30 s.
      await page
        .waitForLoadState("load", { timeout: 30_000 })
        .catch((error: unknown) => {
          if (!(error instanceof Error && error.name === "TimeoutError"))
            throw error;
        });
      await settle(page);
      expect(
        firstText,
        `first text after ${String(firstText)} ms`,
      ).toBeLessThan(FIRST_TEXT_MS);
      expect(await layoutShift(page)).toBeLessThanOrEqual(CLS_LIMIT);
    });

    test("says something without JavaScript", async ({ browser }) => {
      const context = await browser.newContext({ javaScriptEnabled: false });
      const page = await context.newPage();
      await fakeServer(page, target.server);
      await page.goto(target.url);
      expect(await hasText(page), "the page is blank without JavaScript").toBe(
        true,
      );
      await context.close();
    });

    test("is not blank when its scripts fail to load", async ({ page }) => {
      await page.route(/\.m?js(\?.*)?$/, (route) => route.abort());
      await fakeServer(page, target.server);
      await page.goto(target.url);
      expect(await hasText(page), "the page is blank when scripts fail").toBe(
        true,
      );
    });

    test("still opens offline once installed", async ({
      page,
      context,
      browserName,
    }) => {
      test.skip(
        browserName === "firefox" && target.server !== undefined,
        "Firefox sends routed requests past the service worker, so the " +
          "stand-in server rules out offline",
      );
      await fakeServer(page, target.server);
      await page.goto(target.url);
      await settle(page);
      const controlled = await page.evaluate(async () => {
        if (!("serviceWorker" in navigator)) return false;
        await Promise.race([
          navigator.serviceWorker.ready,
          new Promise((resolve) => setTimeout(resolve, 2000)),
        ]);
        return navigator.serviceWorker.controller !== null;
      });
      test.skip(
        !controlled,
        "no service worker controls this page yet (PWA, #139)",
      );
      await context.setOffline(true);
      await page.reload();
      // Offline the app falls back to the demo, and the sign-in and set-up
      // screens redirect home: the router renders one empty frame between
      // the two, so wait for the text instead of sampling once.
      await expect(
        page.locator("body"),
        "the page is blank offline",
      ).toHaveText(/\S/, {
        useInnerText: true,
        timeout: 5000,
      });
    });
  });
}
