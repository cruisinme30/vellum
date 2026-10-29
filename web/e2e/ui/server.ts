// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

import { ADA, driveAnswer } from "./drive-fixture";

/**
 * What a target needs the server to say. Without a server the app runs the
 * Northstar demo, and the sign-in and set-up screens send people home, so
 * those screens are audited against a stand-in for `vellum serve`.
 *
 * - `signed-out`: an existing workspace with nobody signed in.
 * - `new`: a workspace that still needs its first administrator.
 * - `drive`: Ada is signed in, with a Northstar My Drive and a shared drive
 *   (drive-fixture.ts), for the Drive screens (ADR 0052).
 */
export type ServerState = "signed-out" | "new" | "drive";

const STATUS: Record<ServerState, object> = {
  "signed-out": { needsSetup: false, credential: "anonymous", scopes: [] },
  new: { needsSetup: true, credential: "anonymous", scopes: [] },
  drive: { needsSetup: false, credential: "session", scopes: [], user: ADA },
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
  // The organisation's look (ADR 0007): the defaults, as a new server has.
  await context.route("**/api/v1/appearance", (route) => route.fulfill({ json: {} }));
  if (state !== "drive") return;
  await context.route("**/api/v1/**", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/v1/auth/status" || url.pathname === "/api/v1/appearance") {
      return route.fallback();
    }
    const json = driveAnswer(url.pathname);
    return json === undefined ? route.fulfill({ status: 404, json: {} }) : route.fulfill({ json });
  });
}
