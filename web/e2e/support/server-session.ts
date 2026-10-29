// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

import { fakeDrive } from "./fake-drive";

const USER = {
  id: "01M6409GN2EYWS8Z7NFQQ3JNWZ",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: true,
};

/** A signed-in session on a server whose drive is faked. */
export async function signedIn(page: Page) {
  await page.route("**/api/v1/auth/status", (route) =>
    route.fulfill({
      json: { needsSetup: false, credential: "session", scopes: ["read"], user: USER },
    }),
  );
  await page.route("**/api/csrf", (route) =>
    route.fulfill({ json: { token: "t", header: "x-csrf-token" } }),
  );
  await page.route("**/api/v1/collections", (route) =>
    route.fulfill({ json: { collections: [] } }),
  );
  return fakeDrive(page, USER.id);
}
