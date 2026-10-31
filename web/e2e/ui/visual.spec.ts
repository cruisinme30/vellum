// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { expect, test } from "./fixtures";
import { openPanel, settle } from "./harness";
import { fakeServer } from "./server";
import { TARGETS } from "./targets";

/**
 * Visual regression (ADR 0031, D125): each stable screen in light and dark,
 * compared with its baseline at the brief's 0.1 % pixel threshold.
 * Baselines depend on fonts and the browser build, so they are made and
 * checked only inside the pinned Playwright container (UI_VISUAL=1);
 * `scripts/ci/ui-baselines.sh` refreshes them.
 */
test.skip(
  process.env["UI_VISUAL"] !== "1",
  "visual baselines run in the Playwright container only",
);

for (const target of TARGETS.filter((candidate) => candidate.visual)) {
  for (const scheme of ["light", "dark"] as const) {
    test(`${target.name} looks the same (${scheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
      await fakeServer(page, target.server);
      await page.goto(target.url);
      await settle(page);
      await openPanel(page, target);
      const name = `${target.name.replaceAll(/[^\w.-]+/g, "_")}-${scheme}.png`;
      await expect(page).toHaveScreenshot(name, { fullPage: true });
    });
  }
}
