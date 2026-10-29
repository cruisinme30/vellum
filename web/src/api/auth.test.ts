// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { createAuthApi } from "./auth";
import { ApiError, type Http } from "./http";

const USER = {
  id: "u1",
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: true,
};

function fakeHttp(overrides: Partial<Http> = {}): Http & { sent: unknown[][] } {
  const sent: unknown[][] = [];
  return {
    sent,
    get: vi.fn(() =>
      Promise.resolve({ needsSetup: false, credential: "session", scopes: [], user: USER }),
    ),
    send: vi.fn((...call: unknown[]) => {
      sent.push(call);
      return Promise.resolve({ user: USER, sessionId: "s1" });
    }),
    forgetSession: vi.fn(),
    ...overrides,
  } as Http & { sent: unknown[][] };
}

describe("the auth API", () => {
  it("reads the status", async () => {
    await expect(createAuthApi(fakeHttp()).status()).resolves.toMatchObject({ user: USER });
  });

  it("treats no server, a 404 or a non-Vellum answer as demo mode", async () => {
    const offline = fakeHttp({ get: () => Promise.reject(new ApiError("network", "offline")) });
    await expect(createAuthApi(offline).status()).resolves.toBeUndefined();
    const missing = fakeHttp({
      get: () => Promise.reject(new ApiError("not_found", "no such API route", { status: 404 })),
    });
    await expect(createAuthApi(missing).status()).resolves.toBeUndefined();
    const html = fakeHttp({ get: <T>() => Promise.resolve("<!doctype html>" as T) });
    await expect(createAuthApi(html).status()).resolves.toBeUndefined();
  });

  it("passes a real server error on", async () => {
    const broken = fakeHttp({
      get: () => Promise.reject(new ApiError("internal", "boom", { status: 500 })),
    });
    await expect(createAuthApi(broken).status()).rejects.toThrow("boom");
  });

  it("signs in and out, forgetting the CSRF token each time", async () => {
    const forgetSession = vi.fn();
    const http = fakeHttp({ forgetSession });
    const auth = createAuthApi(http);
    await expect(auth.signIn("priya@northstar.dev", "pw")).resolves.toEqual(USER);
    expect(http.sent[0]).toEqual([
      "POST",
      "/auth/sign-in",
      { email: "priya@northstar.dev", password: "pw" },
    ]);
    await auth.signOut();
    expect(http.sent[1]).toEqual(["POST", "/auth/sign-out"]);
    expect(forgetSession).toHaveBeenCalledTimes(2);
  });

  it("returns no user when sign-in answers without one", async () => {
    const http = fakeHttp({ send: <T>() => Promise.resolve({ user: undefined } as T) });
    await expect(createAuthApi(http).signIn("a@b.c", "pw")).resolves.toBeUndefined();
  });

  it("sets up the first administrator, then signs them in", async () => {
    const http = fakeHttp();
    const input = {
      name: "Priya Raman",
      email: "priya@northstar.dev",
      password: "a long password",
    };
    await expect(createAuthApi(http).setUp(input)).resolves.toEqual(USER);
    expect(http.sent.map((call) => call[1])).toEqual(["/api/v1/setup", "/auth/sign-in"]);
    expect(http.sent[0]?.[2]).toEqual(input);
  });
});
