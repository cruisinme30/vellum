// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const GATEWAY = "/p/01JB3KA1C2D3E4F5G6H7J8K9MN";
const PRD = "/p/01JB3KB1C2D3E4F5G6H7J8K9MT";

async function expectNoViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(results.violations).toEqual([]);
}

const editorOf = (page: Page) => page.getByRole("textbox", { name: "Page body" });

/** Every Yjs update in the page cache, decoded loosely as text. */
async function storedText(): Promise<string> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("vellum-northstar");
    request.addEventListener("success", () => {
      resolve(request.result);
    });
    request.addEventListener("error", () => {
      reject(new Error("the page cache did not open"));
    });
  });
  const rows = await new Promise<{ update: Uint8Array }[]>((resolve) => {
    const request = database.transaction("documentUpdates").objectStore("documentUpdates").getAll();
    request.addEventListener("success", () => {
      resolve(request.result as { update: Uint8Array }[]);
    });
  });
  database.close();
  const decoder = new TextDecoder();
  return rows.map((row) => decoder.decode(row.update)).join("");
}

/** Puts the caret on a new empty paragraph after the page's first one. */
async function newLine(page: Page) {
  await editorOf(page).locator("p").first().click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
}

for (const [name, path] of [
  ["home", "/"],
  ["search", "/search?q=gateway"],
  ["help", "/help"],
  ["a page with every block", PRD],
] as const) {
  test(`${name} has no WCAG 2.2 AA violations`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
    if (path.startsWith("/p/")) {
      await expect(editorOf(page)).toBeVisible();
    }
    await expectNoViolations(page);
  });
}

test("every theme passes the contrast checks", async ({ page }) => {
  await page.goto(GATEWAY);
  await expect(editorOf(page)).toBeVisible();
  for (const theme of ["Dark", "High contrast", "Sepia", "Dim"]) {
    await page.getByRole("button", { name: "Theme" }).click();
    await page.getByRole("radio", { name: theme }).check();
    await page.keyboard.press("Escape");
    await expectNoViolations(page);
  }
});

test("the slash menu inserts a block", async ({ page }) => {
  await page.goto(GATEWAY);
  const editor = editorOf(page);
  await newLine(page);
  await page.keyboard.type("/callout");
  await expect(page.getByRole("listbox", { name: "Blocks" })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("listbox", { name: "Blocks" })).toBeHidden();
  await page.keyboard.type("Inserted from the slash menu");
  await expect(editor.getByRole("note").last()).toContainText("Inserted from the slash menu");
});

test("two tabs edit the same page together", async ({ page, context }) => {
  const other = await context.newPage();
  await page.goto(GATEWAY);
  await other.goto(GATEWAY);
  await expect(editorOf(other)).toBeVisible();
  await newLine(page);
  await page.keyboard.type("Typed in the first tab");
  await expect(editorOf(other)).toContainText("Typed in the first tab");
});

test("edits survive a reload", async ({ page }) => {
  await page.goto(GATEWAY);
  await newLine(page);
  // One insertion is one update, so the text is stored in one piece.
  await page.keyboard.insertText("Kept offline");
  await expect(editorOf(page)).toContainText("Kept offline");
  // Updates reach IndexedDB asynchronously; reload once the text is stored.
  await expect.poll(() => page.evaluate(storedText)).toContain("Kept offline");
  await page.reload();
  await expect(editorOf(page)).toContainText("Kept offline");
});

test("the page tree reorders pages from the keyboard", async ({ page }) => {
  await page.goto(GATEWAY);
  const tree = page.getByRole("tree").first();
  const row = tree.getByRole("treeitem", { name: /Gateway/ }).first();
  await row.focus();
  const before = await row.getAttribute("aria-posinset");
  await page.keyboard.press("Alt+Shift+ArrowDown");
  await expect(row).not.toHaveAttribute("aria-posinset", before ?? "");
  await expect(row).toBeFocused();
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test("navigation moves to a bottom bar", async ({ page }) => {
    await page.goto("/");
    const bar = page.getByRole("navigation", { name: "Main" });
    await expect(bar).toBeVisible();
    const box = await bar.boundingBox();
    expect(box?.y ?? 0).toBeGreaterThan(600);
    await expectNoViolations(page);
  });

  test("long unbroken text never scrolls the page sideways", async ({ page }) => {
    await page.goto(GATEWAY);
    const title = page.getByRole("textbox", { name: "Page title" });
    await title.fill(`https://example.com/${"a".repeat(300)} 長い見出し😀`);
    await title.press("Enter");
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});
