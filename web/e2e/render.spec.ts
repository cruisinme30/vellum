// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The one conditional skip below says why: these run at laptop size only.
/* eslint-disable sonarjs/no-skipped-tests */

// Server-rendered blocks in the app (issue #522, ADR 0068): the reading
// view and the editor show every drawn block from the server's renderer,
// in every theme, with no overflow, and pass axe. The server's answers come
// from render-fixture.gen.json, which a vellum-cli test regenerates from
// the real renderer, so these pages are what `vellum serve` sends.

import { readFileSync } from "node:fs";

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { fakeCollab } from "./support/fake-collab";
import { signedIn } from "./support/server-session";

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

interface Fixture {
  readonly stylesheet: unknown;
  readonly page: { readonly html: string };
  readonly blocks: Readonly<Record<string, unknown>>;
}

const FIXTURE = JSON.parse(
  readFileSync(new URL("support/render-fixture.gen.json", import.meta.url), "utf8"),
) as Fixture;

const PAGE = "01M6400000PAGE00ROADMAP001";
const THEMES = [
  ["light", "light"],
  ["dark", "dark"],
  ["high-contrast", "light"],
  ["sepia", "light"],
  ["dim", "dark"],
] as const;
/** The blocks issue #522 asks to see drawn, by the class the renderer gives them. */
const DRAWN = [
  ".vl-chart svg",
  ".vl-timeline",
  ".vl-roadmap svg",
  ".vl-risk svg",
  ".vl-board",
  ".vl-datatable, .vl-table-wrap table",
  ".vl-architecture svg",
  ".vl-diagram svg",
  ".vl-calculator",
];

/** Answers the render routes from the fixture; counts the requests. */
async function fakeRenderer(page: Page, failBlock?: string) {
  const counts = { page: 0, block: 0 };
  await page.route("**/api/v1/render/stylesheet", (route) =>
    route.fulfill({ json: FIXTURE.stylesheet }),
  );
  await page.route(`**/api/v1/pages/${PAGE}/render`, (route) => {
    counts.page += 1;
    return route.fulfill({ json: FIXTURE.page });
  });
  await page.route("**/api/v1/render/block", (route) => {
    counts.block += 1;
    const { lang } = route.request().postDataJSON() as { lang: string };
    if (lang === failBlock) {
      return route.fulfill({
        status: 503,
        json: {
          code: "unavailable",
          message: "The renderer is busy.",
          retryable: false,
          request_id: "req-render-1",
        },
        headers: { "retry-after": "120" },
      });
    }
    const drawn = FIXTURE.blocks[lang];
    return drawn === undefined
      ? route.fulfill({ status: 404, json: { code: "not_found", message: "no fixture" } })
      : route.fulfill({ json: drawn });
  });
  return counts;
}

/** Fails when anything in `selector` sticks out of it sideways. */
async function expectNoOverflow(page: Page, selector: string) {
  const sticking = await page.locator(selector).evaluate((root) => {
    const bounds = root.getBoundingClientRect();
    return [...root.querySelectorAll("*")]
      .filter((element) => {
        // Children of a block that scrolls on its own are fine.
        const scroller = element.parentElement?.closest(".vl-figure, .vl-board, .vl-table-wrap");
        if (scroller !== null && scroller !== undefined && root.contains(scroller)) return false;
        const box = element.getBoundingClientRect();
        return box.width > 0 && (box.left < bounds.left - 1 || box.right > bounds.right + 1);
      })
      .map((element) => `${element.tagName}.${element.getAttribute("class") ?? ""}`);
  });
  expect(sticking).toEqual([]);
}

for (const [theme, scheme] of THEMES) {
  test(`the reading view draws every block in the ${theme} theme`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.addInitScript(
      `new MutationObserver((_, o) => { if (document.documentElement) { document.documentElement.dataset.theme = ${JSON.stringify(theme)}; o.disconnect(); } }).observe(document, { childList: true });`,
    );
    await signedIn(page);
    await fakeCollab(page);
    await fakeRenderer(page);
    await page.goto(`/p/${PAGE}?view=read`);
    const reading = page.getByRole("region", { name: "Rendered page" });
    await expect(reading.getByRole("heading", { name: "Every block" })).toBeVisible();
    for (const selector of DRAWN) {
      await expect(reading.locator(selector).first(), selector).toBeVisible();
    }
    // Styled by the server's theme, scoped to the rendered page.
    const chart = await reading.evaluate((element) =>
      getComputedStyle(element).getPropertyValue("--vl-chart-1").trim(),
    );
    expect(chart).toMatch(/^#[\da-f]{6}$/i);
    await expectNoOverflow(page, ".vl-reading");
    const axe = await new AxeBuilder({ page })
      .include(".vl-reading")
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    expect(axe.violations).toEqual([]);
    await testInfo.attach(`reading-${theme}`, {
      body: await reading.screenshot({ animations: "disabled" }),
      contentType: "image/png",
    });
  });
}

test("a theme change re-themes drawn blocks without asking the server again", async ({ page }) => {
  await signedIn(page);
  await fakeCollab(page);
  const counts = await fakeRenderer(page);
  await page.goto(`/p/${PAGE}?view=read`);
  const bar = page.locator(".vl-reading .vl-chart svg rect").first();
  await expect(bar).toBeVisible();
  const fill = () => bar.evaluate((element) => getComputedStyle(element).fill);
  const light = await fill();
  await page.evaluate(() => {
    document.documentElement.dataset["theme"] = "dark";
  });
  await expect.poll(fill).not.toBe(light);
  expect(counts.page).toBe(1);
});

test("calculators in the reading view recompute as a slider moves", async ({ page }) => {
  await signedIn(page);
  await fakeCollab(page);
  await fakeRenderer(page);
  await page.goto(`/p/${PAGE}?view=read`);
  const calculator = page.locator(".vl-reading .vl-calculator");
  await expect(calculator.locator(".vl-calc-result output")).toHaveText(/1,200/);
  await calculator.getByLabel("Units").fill("250");
  await expect(calculator.locator(".vl-calc-result output")).toHaveText(/3,000/);
});

test("the editor shows server-drawn blocks, with their source a click away", async ({ page }) => {
  await signedIn(page);
  const mermaid = "flowchart LR\n  A[Request] --> B{Allowed?}\n";
  await fakeCollab(page, {
    fences: [
      { lang: "mermaid", source: mermaid },
      { lang: "timeline", source: "2026-10-01 | Beta opens\n" },
    ],
  });
  const counts = await fakeRenderer(page);
  await page.goto(`/p/${PAGE}`);
  const mermaidBlock = page.locator('.vl-fence[data-lang="mermaid"]');
  await expect(mermaidBlock.locator(".vl-fence-render .vl-diagram svg")).toBeVisible();
  await expect(
    page.locator('.vl-fence[data-lang="timeline"] .vl-fence-render .vl-timeline'),
  ).toBeVisible();
  expect(counts.block).toBeGreaterThanOrEqual(2);

  const source = mermaidBlock.getByLabel("mermaid source");
  await expect(source).toBeHidden();
  await mermaidBlock.getByRole("button", { name: "Source" }).click();
  await expect(source).toBeVisible();
  await expect(source).toContainText("A[Request]");
  await expectNoOverflow(page, ".vl-prose");
  const axe = await new AxeBuilder({ page })
    .include(".vl-fence")
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(axe.violations).toEqual([]);
});

test("a block the server cannot draw says why, with Copy diagnostic", async ({
  page,
  browserName,
  context,
}) => {
  if (browserName === "chromium") {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  }
  await signedIn(page);
  await fakeCollab(page, {
    fences: [{ lang: "roadmap", source: "GA | 2026-11-24 | milestone\n" }],
  });
  await fakeRenderer(page, "roadmap");
  await page.goto(`/p/${PAGE}`);
  const block = page.locator('.vl-fence[data-lang="roadmap"]');
  await expect(block.getByRole("alert")).toContainText(
    "The block could not be drawn: The renderer is busy. (request req-render-1)",
  );
  await block.getByRole("button", { name: "Copy diagnostic" }).click();
  // Copied where the clipboard is allowed; shown to copy by hand elsewhere.
  const copied = block.getByRole("status").filter({ hasText: "Diagnostic copied" });
  const manual = block.getByRole("textbox", { name: "Diagnostic to copy" });
  await expect(copied.or(manual)).toBeVisible();
  if (browserName === "chromium") {
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text).toContain("Request ID: req-render-1");
    expect(text).toContain("Route: POST /api/v1/render/block");
  }
});
