// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

/**
 * What a target needs the server to say. Without a server the app runs the
 * Northstar demo, and the sign-in and set-up screens send people home, so
 * those screens are audited against a stand-in for `vellum serve`.
 *
 * - `signed-out`: an existing workspace with nobody signed in.
 * - `new`: a workspace that still needs its first administrator.
 */
export type ServerState = "signed-out" | "new";

const STATUS: Record<ServerState, object> = {
  "signed-out": { needsSetup: false, credential: "anonymous", scopes: [] },
  new: { needsSetup: true, credential: "anonymous", scopes: [] },
};

/**
 * Answers the requests the app makes before anyone signs in. Routes are set
 * on the context, so they also answer requests a service worker forwards.
 */
export async function fakeServer(page: Page, state: ServerState | undefined): Promise<void> {
  if (state === undefined) return;
  const context = page.context();
  await context.route("**/api/v1/auth/status", (route) => route.fulfill({ json: STATUS[state] }));
  await context.route("**/api/csrf", (route) =>
    route.fulfill({ json: { token: "t", header: "x-csrf-token" } }),
  );
}
