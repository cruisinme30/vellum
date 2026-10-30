// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

import { ADA, driveAnswer } from "./drive-fixture";
import { APPEARANCE, NORTHSTAR_NOW, northstarServer } from "./northstar-server";
import { workRoutes } from "./work-server";

/**
 * What a target needs the server to say. Without a server the app runs the
 * Northstar demo, and the sign-in and set-up screens send people home, so
 * those screens are audited against a stand-in for `vellum serve`.
 *
 * - `signed-out`: an existing workspace with nobody signed in.
 * - `new`: a workspace that still needs its first administrator.
 * - `drive`: Ada is signed in, with a Northstar My Drive and a shared drive
 *   (drive-fixture.ts), for the Drive screens (ADR 0052).
 * - `work`: a signed-in member, with the Work API answering (issue #504).
 * - `signed-in`: Priya, an administrator, in the Northstar workspace with
 *   every read API answering (northstar-server.ts, D285), for Work, My
 *   Work, Books, admin and settings.
 */
export type ServerState = "signed-out" | "new" | "drive" | "work" | "signed-in";

const STATUS: Record<Exclude<ServerState, "signed-in">, object> = {
  "signed-out": { needsSetup: false, credential: "anonymous", scopes: [] },
  new: { needsSetup: true, credential: "anonymous", scopes: [] },
  drive: { needsSetup: false, credential: "session", scopes: [], user: ADA },
  work: {
    needsSetup: false,
    credential: "session",
    scopes: ["read", "write", "work"],
    user: {
      id: "01JNORTHSTARUSER0000000000",
      email: "mara@northstar.dev",
      name: "Mara Lind",
      handle: "mara",
      admin: false,
    },
  },
};

/**
 * Answers the requests the app makes before anyone signs in. Routes are set
 * on the context, so they also answer requests a service worker forwards.
 */
export async function fakeServer(page: Page, state: ServerState | undefined): Promise<void> {
  if (state === undefined) return;
  const context = page.context();
  if (state === "work") await workRoutes(context);
  if (state === "signed-in") {
    // Relative times ("2 days ago") and today's buckets stay the same on any day.
    await page.clock.setFixedTime(NORTHSTAR_NOW);
    await northstarServer(context);
    return;
  }
  await context.route("**/api/v1/auth/status", (route) => route.fulfill({ json: STATUS[state] }));
  // The organisation's look is fetched before anyone signs in (R-TH-1).
  await context.route("**/api/v1/appearance", (route) => route.fulfill({ json: APPEARANCE }));
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
