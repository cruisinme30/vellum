// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Video 4 of 10: a collection and its board view
// (docs/user.book/data/views.md).

import { expect, test } from "./tour";

const ISSUES = "/data/01JB3KD0COLL0ISSUES0000001";

test("a collection and its board view", async ({ page, tour }) => {
  await tour.scene("Data lists every collection you can open.", async () => {
    await page.goto("/data");
    await expect(
      page.getByRole("heading", { level: 1, name: "Data" }),
    ).toBeVisible();
  });
  await tour.scene(
    "A collection is a table of rows, stored as files.",
    async () => {
      await page.goto(`${ISSUES}?view=All+issues`);
      await expect(page.getByRole("grid")).toBeVisible();
    },
  );
  await tour.scene("The same rows as a board, grouped by status.", async () => {
    await page.goto(`${ISSUES}?view=Board`);
    await expect(page.getByRole("region").first()).toBeVisible();
  });
});
