// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { HealthMonitor, retryAfterSeconds } from "./health";
import { ApiError, createHttp, queryString } from "./http";

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  Response.json(body, { status, headers });

/** A fake server: answers each request from `routes`, recording what was sent. */
function fakeFetch(routes: (method: string, url: string, init: RequestInit) => Response) {
  const calls: { method: string; url: string; init: RequestInit }[] = [];
  const fetch = vi.fn((input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = input instanceof Request ? input.url : String(input);
    const method = init.method ?? "GET";
    calls.push({ method, url, init });
    return Promise.resolve(routes(method, url, init));
  });
  return { fetch: fetch as unknown as typeof globalThis.fetch, calls };
}

const header = (init: RequestInit, name: string) => (init.headers as Record<string, string>)[name];

describe("queryString", () => {
  it("repeats list keys and leaves out empty values", () => {
    expect(queryString({ filter: ["a", "b c"], view: "Board", limit: 5, gone: undefined })).toBe(
      "?filter=a&filter=b+c&view=Board&limit=5",
    );
    expect(queryString({})).toBe("");
  });
});

describe("the HTTP client", () => {
  it("reads JSON with the session cookie", async () => {
    const { fetch, calls } = fakeFetch(() => json({ ok: true }));
    const http = createHttp({ fetch });
    await expect(http.get("/api/v1/thing", { q: "x" })).resolves.toEqual({ ok: true });
    expect(calls[0]?.url).toBe("/api/v1/thing?q=x");
    expect(calls[0]?.init.credentials).toBe("same-origin");
  });

  it("sends writes with the CSRF token, an idempotency key and a JSON body", async () => {
    const { fetch, calls } = fakeFetch((method, url) =>
      url === "/api/csrf" ? json({ token: "t0k", header: "x-csrf-token" }) : json({ method }),
    );
    const http = createHttp({ fetch, idempotencyKey: () => "key-1" });
    await expect(http.send("PATCH", "/api/v1/x", { a: 1 })).resolves.toEqual({ method: "PATCH" });
    await http.send("DELETE", "/api/v1/x");
    const write = calls.find((c) => c.method === "PATCH");
    expect(header(write?.init ?? {}, "x-csrf-token")).toBe("t0k");
    expect(header(write?.init ?? {}, "idempotency-key")).toBe("key-1");
    expect(header(write?.init ?? {}, "content-type")).toBe("application/json");
    expect(write?.init.body).toBe('{"a":1}');
    // The token is fetched once and reused.
    expect(calls.filter((c) => c.url === "/api/csrf")).toHaveLength(1);
    const remove = calls.find((c) => c.method === "DELETE");
    expect(remove?.init.body).toBeUndefined();
  });

  it("sends writes without a token before anyone signs in", async () => {
    const { fetch, calls } = fakeFetch((_method, url) =>
      url === "/api/csrf"
        ? json(
            {
              code: "unauthenticated",
              message: "no browser session",
              retryable: false,
              request_id: "r",
            },
            401,
          )
        : new Response(undefined, { status: 204 }),
    );
    const http = createHttp({ fetch });
    await expect(http.send("POST", "/auth/sign-out")).resolves.toBeUndefined();
    expect(header(calls[1]?.init ?? {}, "x-csrf-token")).toBeUndefined();
  });

  it("fetches a new token once when the session changed", async () => {
    let valid = "old";
    const { fetch, calls } = fakeFetch((_method, url, init) => {
      if (url === "/api/csrf") return json({ token: valid });
      return header(init, "x-csrf-token") === valid
        ? json({ done: true })
        : json(
            {
              code: "permission_denied",
              message: "missing or invalid CSRF token; reload the page",
              retryable: false,
              request_id: "r1",
            },
            403,
          );
    });
    const http = createHttp({ fetch });
    await expect(http.send("POST", "/api/v1/x", {})).resolves.toEqual({ done: true });
    valid = "new";
    await expect(http.send("POST", "/api/v1/x", {})).resolves.toEqual({ done: true });
    expect(calls.filter((c) => c.url === "/api/csrf")).toHaveLength(2);
  });

  it("forgets the token when told the session changed", async () => {
    const { fetch, calls } = fakeFetch((_method, url) =>
      url === "/api/csrf" ? json({ token: "a" }) : json({}),
    );
    const http = createHttp({ fetch });
    await http.send("POST", "/api/v1/x");
    http.forgetSession();
    await http.send("POST", "/api/v1/x");
    expect(calls.filter((c) => c.url === "/api/csrf")).toHaveLength(2);
  });

  it("keeps a permission error that is not about CSRF", async () => {
    const { fetch } = fakeFetch((_method, url) =>
      url === "/api/csrf"
        ? json({ token: "a" })
        : json(
            {
              code: "permission_denied",
              message: "You cannot edit this",
              retryable: false,
              request_id: "r2",
            },
            403,
          ),
    );
    const error: unknown = await createHttp({ fetch })
      .send("POST", "/api/v1/x")
      .catch((error_: unknown) => error_);
    expect(error).toMatchObject({ code: "permission_denied", status: 403, requestId: "r2" });
  });

  it("turns the server's error body into an ApiError", async () => {
    const { fetch } = fakeFetch(() =>
      json(
        {
          code: "conflict",
          message: "The file changed on disk",
          retryable: true,
          request_id: "01J",
        },
        409,
      ),
    );
    const error: unknown = await createHttp({ fetch })
      .get("/api/v1/x")
      .catch((error_: unknown) => error_);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      code: "conflict",
      message: "The file changed on disk",
      status: 409,
      retryable: true,
      requestId: "01J",
    });
  });

  it("maps unknown codes and bodies without JSON", async () => {
    const odd = fakeFetch(() =>
      json({ code: "weird", message: "Odd" }, 400, { "x-request-id": "h1" }),
    );
    await expect(createHttp({ fetch: odd.fetch }).get("/x")).rejects.toMatchObject({
      code: "internal",
      requestId: "h1",
      retryable: false,
    });
    const down = fakeFetch(
      () => new Response("<html>", { status: 502, statusText: "Bad Gateway" }),
    );
    await expect(createHttp({ fetch: down.fetch }).get("/x")).rejects.toMatchObject({
      code: "unavailable",
      message: "Bad Gateway",
      retryable: true,
    });
    const blank = fakeFetch(
      () => new Response("", { status: 418, headers: { "x-request-id": "h2" } }),
    );
    await expect(createHttp({ fetch: blank.fetch }).get("/x")).rejects.toMatchObject({
      code: "internal",
      requestId: "h2",
    });
  });

  it("reports a network failure as retryable", async () => {
    const fetch = vi.fn(() => Promise.reject(new TypeError("Failed to fetch")));
    await expect(
      createHttp({ fetch: fetch as unknown as typeof globalThis.fetch }).get("/x"),
    ).rejects.toMatchObject({ code: "network", retryable: true, status: 0 });
    // A token request that cannot reach the server leaves the write without one.
    await expect(
      createHttp({ fetch: fetch as unknown as typeof globalThis.fetch }).send("POST", "/x"),
    ).rejects.toMatchObject({ code: "network" });
  });

  it("prefixes paths with the base and uses the global fetch by default", async () => {
    const spy = vi.spyOn(globalThis, "fetch").mockResolvedValue(json({ base: true }));
    await expect(createHttp({ base: "https://v.example" }).get("/api/x")).resolves.toEqual({
      base: true,
    });
    expect(spy).toHaveBeenCalledWith("https://v.example/api/x", expect.anything());
    spy.mockRestore();
  });
});

describe("when part of the server is down (503)", () => {
  it("retries a write once with the same idempotency key after Retry-After", async () => {
    let answers = 0;
    const { fetch, calls } = fakeFetch((_method, url) => {
      if (url === "/api/csrf") return json({ token: "t" });
      answers += 1;
      return answers === 1
        ? json({ code: "unavailable", message: "bucket down" }, 503, { "retry-after": "2" })
        : json({ saved: true });
    });
    const waits: number[] = [];
    const health = new HealthMonitor();
    const http = createHttp({
      fetch,
      idempotencyKey: () => "key-9",
      health,
      sleep: (ms) => {
        waits.push(ms);
        return Promise.resolve();
      },
    });
    await expect(http.send("PUT", "/api/v1/pages/1", { a: 1 })).resolves.toEqual({ saved: true });
    const puts = calls.filter((c) => c.method === "PUT");
    expect(puts).toHaveLength(2);
    expect(puts.map((c) => header(c.init, "idempotency-key"))).toEqual(["key-9", "key-9"]);
    expect(waits).toEqual([2000]);
    expect(health.snapshot().degraded).toBe(false);
  });

  it("gives up after one retry, or at once when asked to wait too long", async () => {
    const health = new HealthMonitor();
    const down = fakeFetch(() =>
      json({ code: "unavailable", message: "bucket down", retryable: true }, 503, {
        "retry-after": "1",
      }),
    );
    const http = createHttp({ fetch: down.fetch, health, sleep: () => Promise.resolve() });
    await expect(http.get("/api/v1/pages/1")).rejects.toMatchObject({
      code: "unavailable",
      status: 503,
    });
    expect(down.calls).toHaveLength(2);
    expect(health.snapshot().degraded).toBe(true);

    const slow = fakeFetch(() =>
      json({ code: "unavailable", message: "x" }, 503, { "retry-after": "60" }),
    );
    const http2 = createHttp({ fetch: slow.fetch, health, sleep: () => Promise.resolve() });
    await expect(http2.get("/api/v1/pages/1")).rejects.toBeInstanceOf(ApiError);
    expect(slow.calls).toHaveLength(1);
  });
});

describe("HealthMonitor", () => {
  it("turns degraded on a 503 and back on the next good answer", () => {
    const health = new HealthMonitor();
    const seen: boolean[] = [];
    const stop = health.subscribe(() => seen.push(health.snapshot().degraded));
    health.report(200);
    health.report(503, 3, 1000);
    expect(health.snapshot()).toEqual({ degraded: true, retryAt: 4000 });
    health.report(503, 3, 1000);
    health.report(500);
    health.report(404);
    stop();
    health.report(503);
    expect(seen).toEqual([true, false]);
  });

  it("reads Retry-After seconds and ignores dates", () => {
    expect(retryAfterSeconds("5")).toBe(5);
    expect(retryAfterSeconds("Wed, 21 Oct 2026 07:28:00 GMT")).toBeUndefined();
    // eslint-disable-next-line unicorn/no-null -- Headers.get answers null for a missing header
    expect(retryAfterSeconds(null)).toBeUndefined();
  });
});
