// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { test as base } from "@playwright/test";

/**
 * The UI suite's `test`: every page opens with window.print stubbed out,
 * because a book's print page calls it on load and a print dialog blocks
 * a headless browser (Firefox hangs until the test times out).
 */
export const test = base.extend({
  page: async ({ page }, provide) => {
    await page.addInitScript({ content: "window.print = () => {};" });
    await provide(page);
  },
});

export { expect } from "@playwright/test";
