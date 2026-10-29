// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { AdminApiError, fetchRequests, fetchTrace, REQUESTS_PATH } from "./api";
import { SUMMARY, TRACE } from "./fixtures";

describe("admin requests API", () => {
  it("encodes every filter in the query string", async () => {
    const doFetch = vi.fn<typeof fetch>(() =>
      Promise.resolve(Response.json({ requests: [SUMMARY] })),
    );
    const rows = await fetchRequests(
      { route: "/a", minDurationMs: 50, errorsOnly: true, id: "req-1", before: 9, limit: 20 },
      doFetch,
    );
    expect(rows).toEqual([SUMMARY]);
    const [url, init] = doFetch.mock.calls[0] ?? [];
    expect(url).toBe(
      `${REQUESTS_PATH}?route=%2Fa&min_duration_ms=50&errors_only=true&id=req-1&before=9&limit=20`,
    );
    expect(new Headers(init?.headers).get("traceparent")).toMatch(/^00-/);
    await fetchRequests({}, doFetch);
    expect(doFetch.mock.calls[1]?.[0]).toBe(REQUESTS_PATH);
  });

  it("fetches one trace", async () => {
    const doFetch = vi.fn(() => Promise.resolve(Response.json(TRACE)));
    expect(await fetchTrace("abc/def", doFetch)).toEqual(TRACE);
    expect(doFetch).toHaveBeenCalledWith(`${REQUESTS_PATH}/abc%2Fdef`, expect.any(Object));
  });

  it("reports HTTP and network failures with the request id", async () => {
    const notFound = vi.fn(() => Promise.resolve(new Response("", { status: 404 })));
    const error = await fetchTrace("x", notFound).catch((error_: unknown) => error_);
    expect(error).toBeInstanceOf(AdminApiError);
    expect((error as AdminApiError).status).toBe(404);
    expect((error as AdminApiError).requestId).toMatch(/-/);
    const offline = vi.fn(() => Promise.reject(new Error("offline")));
    const networkError = await fetchRequests({}, offline).catch((error_: unknown) => error_);
    expect((networkError as AdminApiError).status).toBe(0);
  });
});
