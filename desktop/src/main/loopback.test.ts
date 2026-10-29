// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { waitForRedirect } from "./loopback.js";

describe("waitForRedirect", () => {
  it("resolves the code when the state matches", async () => {
    const redirect = await waitForRedirect("s1");
    expect(redirect.uri).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/callback$/);
    const missing = await fetch(redirect.uri.replace("/callback", "/other"));
    expect(missing.status).toBe(404);
    const page = await fetch(`${redirect.uri}?code=c1&state=s1`);
    expect(page.status).toBe(200);
    expect(page.headers.get("cache-control")).toBe("no-store");
    await expect(redirect.code).resolves.toBe("c1");
    redirect.close();
  });

  it("rejects a mismatched state or an error", async () => {
    const mismatched = await waitForRedirect("s1");
    const page = await fetch(`${mismatched.uri}?code=c1&state=other`);
    expect(page.status).toBe(400);
    await expect(mismatched.code).rejects.toThrow(/did not match/);
    mismatched.close();

    const denied = await waitForRedirect("s1");
    await fetch(`${denied.uri}?error=access_denied&state=s1`);
    await expect(denied.code).rejects.toThrow("access_denied");
    denied.close();
  });

  it("times out and can be cancelled", async () => {
    const slow = await waitForRedirect("s1", 10);
    await expect(slow.code).rejects.toThrow(/timed out/);
    slow.close();
    const cancelled = await waitForRedirect("s1");
    cancelled.close();
    await expect(cancelled.code).rejects.toThrow(/cancelled/);
  });
});
