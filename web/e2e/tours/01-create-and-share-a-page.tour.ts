// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Video 1 of 10: create and share a page (docs/user.book/start/README.md).

import { expect, test } from "./tour";

test("create and share a page", async ({ page, tour }) => {
  await tour.scene(
    "Vellum opens on Home, with the Northstar drives in the sidebar.",
    async () => {
      await page.goto("/");
      await expect(page.getByRole("tree").first()).toBeVisible();
    },
  );
  await tour.scene(
    "Add a page to the Engineering drive and give it a title.",
    async () => {
      await page
        .getByRole("button", { name: "Add a page to Engineering" })
        .click();
      const title = page.getByRole("textbox", { name: "Page title" });
      await title.fill("Gateway launch notes");
      await title.press("Enter");
    },
  );
  await tour.scene(
    "Type the body. It saves as you type: there is no save button.",
    async () => {
      const body = page.getByRole("textbox", { name: "Page body" });
      await body.click();
      await page.keyboard.type(
        "Launch moves to 12 November. Priya owns the checklist.",
        {
          delay: 30,
        },
      );
      await expect(body).toContainText("Priya owns the checklist");
    },
  );
  await tour.scene(
    "The page is in the tree, ready to share from its menu.",
    async () => {
      await expect(
        page
          .getByRole("tree")
          .first()
          .getByRole("treeitem", { name: "Gateway launch notes" }),
      ).toBeVisible();
    },
  );
});
