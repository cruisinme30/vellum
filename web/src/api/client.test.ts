// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { createClient, prepare, VellumApiError } from "./client";

function json(status: number, body: unknown): Response {
  return Response.json(body, { status });
}

function href(url: RequestInfo | URL): string {
  return url instanceof Request ? url.url : url.toString();
}

describe("prepare", () => {
  it("fills path parameters and sends the rest as a query", () => {
    expect(
      prepare("books.diff", { book_id: "b/1", from: "v1", to: "v 2" }),
    ).toEqual({
      method: "GET",
      url: "/v1/books/b%2F1/diff?from=v1&to=v+2",
    });
    expect(
      prepare("books.get_section", { book_id: "b1", section_id: "s 2" }),
    ).toEqual({
      method: "GET",
      url: "/v1/books/b1/sections/s%202",
    });
  });

  it("sends writes as a JSON body", () => {
    const request = prepare("books.add_section", {
      book_id: "b1",
      title: "Runbook",
      place: { at: "end" },
    });
    expect(request.method).toBe("POST");
    expect(request.url).toBe("/v1/books/b1/sections");
    expect(JSON.parse(request.body ?? "")).toEqual({
      book_id: "b1",
      title: "Runbook",
      place: { at: "end" },
    });
  });
});

describe("createClient", () => {
  it("sends a CSRF token with session writes, fetched once", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async (url) => {
      await Promise.resolve();
      return href(url) === "/api/csrf"
        ? json(200, { token: "t1" })
        : json(200, { ok: true });
    });
    const client = createClient({ fetch });
    await client.call(
      "books.add_section",
      { book_id: "b1", title: "A", place: { at: "end" } },
      {
        idempotencyKey: "k1",
      },
    );
    await client.call("books.add_section", {
      book_id: "b1",
      title: "B",
      place: { at: "end" },
    });
    await client.call("books.get", { book_id: "b1" });
    const calls = fetch.mock.calls.map(([url, init]) => ({
      url: href(url),
      headers: new Headers(init?.headers),
    }));
    expect(calls.map((c) => c.url)).toEqual([
      "/api/csrf",
      "/api/v1/books/b1/sections",
      "/api/v1/books/b1/sections",
      "/api/v1/books/b1",
    ]);
    expect(calls[1]?.headers.get("x-csrf-token")).toBe("t1");
    expect(calls[1]?.headers.get("idempotency-key")).toBe("k1");
    expect(calls[2]?.headers.get("idempotency-key")).toBeNull();
    expect(calls[3]?.headers.get("x-csrf-token")).toBeNull();
  });

  it("uses a bearer token instead of the session when given one", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(() =>
      Promise.resolve(json(200, {})),
    );
    const client = createClient({
      fetch,
      token: "vlm_x",
      baseUrl: "https://vellum.northstar.dev/api/",
    });
    await client.call("books.add_section", {
      book_id: "b1",
      title: "A",
      place: { at: "end" },
    });
    const [url, init] = fetch.mock.calls[0] ?? ["", undefined];
    expect(href(url)).toBe(
      "https://vellum.northstar.dev/api/v1/books/b1/sections",
    );
    const headers = new Headers(init?.headers);
    expect(headers.get("authorization")).toBe("Bearer vlm_x");
    expect(headers.get("x-csrf-token")).toBeNull();
  });

  it("turns the gateway's error body into a typed error", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(() =>
      Promise.resolve(
        json(404, {
          code: "not_found",
          message: "book not found",
          retryable: false,
          request_id: "r1",
        }),
      ),
    );
    const client = createClient({ fetch });
    const error: unknown = await client
      .call("books.get", { book_id: "nope" })
      .catch((error_: unknown) => error_);
    expect(error).toBeInstanceOf(VellumApiError);
    expect(error).toMatchObject({
      status: 404,
      code: "not_found",
      requestId: "r1",
      message: "book not found",
    });
  });

  it("reports a response that is not an API error", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(() =>
      Promise.resolve(
        new Response("bad gateway", {
          status: 502,
          headers: { "x-request-id": "r9" },
        }),
      ),
    );
    const error: unknown = await createClient({ fetch })
      .call("books.list", {})
      .catch((error_: unknown) => error_);
    expect(error).toMatchObject({
      status: 502,
      code: "internal",
      retryable: true,
      requestId: "r9",
    });
  });
});
