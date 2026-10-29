// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Page } from "@playwright/test";

import { workRoutes } from "./work-server";

/**
 * What a target needs the server to say. Without a server the app runs the
 * Northstar demo, and the sign-in and set-up screens send people home, so
 * those screens are audited against a stand-in for `vellum serve`.
 *
 * - `signed-out`: an existing workspace with nobody signed in.
 * - `new`: a workspace that still needs its first administrator.
 * - `work`: a signed-in member, with the Work API answering (issue #504).
 */
export type ServerState = "signed-out" | "new" | "work";

const STATUS: Record<ServerState, object> = {
  "signed-out": { needsSetup: false, credential: "anonymous", scopes: [] },
  new: { needsSetup: true, credential: "anonymous", scopes: [] },
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
  await context.route("**/api/v1/auth/status", (route) => route.fulfill({ json: STATUS[state] }));
  await context.route("**/api/csrf", (route) =>
    route.fulfill({ json: { token: "t", header: "x-csrf-token" } }),
  );
}
