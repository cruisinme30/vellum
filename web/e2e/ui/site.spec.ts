// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The checks need the built site; with only the bare landing page they say so.
/* eslint-disable sonarjs/no-skipped-tests */

import { existsSync } from "node:fs";
import path from "node:path";

import type { Page } from "@playwright/test";

import { expect, test } from "./fixtures";
import { SITE_DIR, SITE_PORT } from "./targets";

/**
 * The website's one look and its rich blocks (ADR 0071): every page type
 * carries the same site bar in the same place and colours, in light and
 * dark; diagrams are drawn, fit their column and follow the theme; tab
 * sets work from the keyboard. Runs in every viewport project.
 */

const built = existsSync(
  path.join(SITE_DIR, "docs", "developer", "index.html"),
);
const url = (page: string): string =>
  `http://127.0.0.1:${String(SITE_PORT)}/${page}`;

/**
 * One page of each type: landing, docs home, a guide, a deep chapter, the
 * decisions.
 */
const PAGES = [
  "index.html",
  "docs/index.html",
  "docs/user/index.html",
  "docs/developer/architecture/index.html",
  "docs/decisions/index.html",
] as const;

interface Bar {
  top: number;
  height: number;
  brandLeft: number;
  background: string;
  links: string[];
  current: string[];
}

const bar = (page: Page): Promise<Bar> =>
  page.evaluate(() => {
    const header = document.querySelector<HTMLElement>("header.vellum-site");
    if (header === null) throw new Error("no site bar");
    const box = header.getBoundingClientRect();
    const brand = header
      .querySelector(".vellum-brand")
      ?.getBoundingClientRect();
    const links = [
      ...header.querySelectorAll<HTMLAnchorElement>(".vellum-nav a"),
    ];
    return {
      top: box.top,
      height: box.height,
      brandLeft: brand?.left ?? -1,
      background: getComputedStyle(header).backgroundColor,
      links: links.map((link) => link.textContent.trim()),
      current: links
        .filter((link) => link.getAttribute("aria-current") === "page")
        .map((link) => link.textContent.trim()),
    };
  });

test.describe("site look", () => {
  test.skip(
    !built,
    "needs the built site: run scripts/ci/build-site.sh and set UI_SITE_DIR",
  );

  for (const scheme of ["light", "dark"] as const) {
    test(`every page type has the same site bar in ${scheme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: scheme });
      const bars: Bar[] = [];
      for (const target of PAGES) {
        await page.goto(url(target));
        const theme = await page.evaluate(
          () => document.documentElement.dataset["theme"],
        );
        expect(theme, `${target} theme`).toBe(scheme);
        bars.push(await bar(page));
      }
      const [first, ...rest] = bars;
      if (first === undefined) throw new Error("no pages");
      expect(first.top).toBe(0);
      for (const [index, other] of rest.entries()) {
        const name = PAGES[index + 1];
        expect(other.top, `${String(name)} bar top`).toBe(0);
        expect(
          Math.abs(other.height - first.height),
          `${String(name)} bar height`,
        ).toBeLessThan(1);
        expect(
          Math.abs(other.brandLeft - first.brandLeft),
          `${String(name)} brand`,
        ).toBeLessThan(1);
        expect(other.background, `${String(name)} bar colour`).toBe(
          first.background,
        );
        expect(other.links, `${String(name)} links`).toEqual(first.links);
      }
      expect(bars.map((b) => b.current)).toEqual([
        ["Product"],
        ["Docs"],
        ["User guide"],
        ["Developer guide"],
        ["Decisions"],
      ]);
    });
  }

  test("the bar never covers the guide's sidebar or page, and leaves no gap", async ({
    page,
  }) => {
    await page.goto(url("docs/developer/architecture/index.html"));
    const { barBottom, menuTop, sidebarTop, sidebarShown } =
      await page.evaluate(() => {
        const header = document
          .querySelector("header.vellum-site")
          ?.getBoundingClientRect();
        const menu = document
          .querySelector("#mdbook-menu-bar")
          ?.getBoundingClientRect();
        const sidebar = document.querySelector("#mdbook-sidebar");
        return {
          barBottom: header?.bottom ?? 0,
          menuTop: menu?.top ?? 0,
          sidebarTop: sidebar?.getBoundingClientRect().top ?? 0,
          sidebarShown:
            document.documentElement.classList.contains("sidebar-visible"),
        };
      });
    // The guide's own menu bar sits right under the site bar: not under
    // it, and with no empty strip between them.
    expect(menuTop).toBeGreaterThanOrEqual(barBottom - 1);
    expect(menuTop).toBeLessThanOrEqual(barBottom + 1);
    if (sidebarShown) expect(sidebarTop).toBeGreaterThanOrEqual(barBottom - 1);
  });

  test("diagrams are drawn, fit the page and follow the theme toggle", async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto(url("docs/developer/architecture/data-flow.html"));
    await expect(page.locator("code.language-mermaid")).toHaveCount(0);
    const figures = page.locator(".vl-site-block svg");
    expect(await figures.count()).toBeGreaterThanOrEqual(4);
    const overflow = await page.evaluate(
      () =>
        [...document.querySelectorAll(".vl-site-block")].filter((block) => {
          const svg = block.querySelector("svg");
          return (
            svg !== null &&
            svg.getBoundingClientRect().width >
              block.getBoundingClientRect().width + 1
          );
        }).length,
    );
    expect(overflow, "diagrams wider than the page").toBe(0);

    const surface = (): Promise<string> =>
      page
        .locator(".vl-render")
        .first()
        .evaluate((block) =>
          getComputedStyle(block).getPropertyValue("--vl-surface").trim(),
        );
    const light = await surface();
    await page.locator("header.vellum-site .theme-toggle").click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    expect(await surface()).not.toBe(light);
  });

  test("callouts are styled blocks, not plain paragraphs", async ({ page }) => {
    await page.goto(url("docs/user/writing/containers.html"));
    const callout = page.locator(".vl-callout-info").first();
    await expect(callout).toBeVisible();
    const padding = await callout.evaluate(
      (element) => getComputedStyle(element).paddingLeft,
    );
    expect(Number.parseFloat(padding)).toBeGreaterThan(4);
  });

  test("a tab set works from the keyboard", async ({ page }) => {
    await page.goto(url("docs/admin/deploy/index.html"));
    const tabs = page.getByRole("tab");
    await expect(tabs).toHaveCount(3);
    await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tabpanel")).toHaveCount(1);
    await tabs.nth(0).focus();
    await page.keyboard.press("ArrowRight");
    await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
    await expect(tabs.nth(1)).toBeFocused();
    await expect(page.getByRole("tabpanel")).toContainText("docker compose");
    await page.keyboard.press("End");
    await expect(tabs.nth(2)).toHaveAttribute("aria-selected", "true");
  });
});
