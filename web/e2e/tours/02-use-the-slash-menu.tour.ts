// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Video 2 of 10: use the slash menu (docs/user.book/start/README.md#slash-menu).

import { expect, test } from "./tour";

const GATEWAY = "/p/01JB3KA1C2D3E4F5G6H7J8K9MN";

test("use the slash menu", async ({ page, tour }) => {
  const editor = page.getByRole("textbox", { name: "Page body" });
  await tour.scene("Open a page and start a new line.", async () => {
    await page.goto(GATEWAY);
    await editor.locator("p").first().click();
    await page.keyboard.press("End");
    await page.keyboard.press("Enter");
  });
  await tour.scene("Type a slash, then a few letters to filter the blocks.", async () => {
    await page.keyboard.type("/callout", { delay: 120 });
    await expect(page.getByRole("listbox", { name: "Blocks" })).toBeVisible();
  });
  await tour.scene("Press Enter to insert the highlighted block, then type.", async () => {
    await page.keyboard.press("Enter");
    await page.keyboard.type("Freeze the gateway config on 10 November.", { delay: 40 });
    await expect(editor.getByRole("note").last()).toContainText("Freeze the gateway config");
  });
});
