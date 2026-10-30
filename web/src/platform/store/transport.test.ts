// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { httpTransport, parsePull, parsePush, SyncError } from "./transport";

function respond(status: number, body: unknown) {
  return vi.fn<typeof fetch>(() =>
    Promise.resolve(Response.json(body, { status })),
  );
}

// The lint rules forbid the null literal; the network can still send it.
const NULL: unknown = JSON.parse("null");

describe("httpTransport", () => {
  it("posts mutations with tracing headers and the session cookie", async () => {
    const fetchMock = respond(200, { lastMutationId: 3, rejected: [] });
    const transport = httpTransport({
      baseUrl: "https://northstar.dev",
      fetch: fetchMock,
    });
    const result = await transport.push({ clientId: "c", mutations: [] });
    expect(result).toEqual({ lastMutationId: 3, rejected: [] });
    const [url, init] = fetchMock.mock.calls[0] ?? [];
    expect(url).toBe("https://northstar.dev/api/v1/sync/push");
    expect(init?.method).toBe("POST");
    expect(init?.credentials).toBe("include");
    const headers = new Headers(init?.headers);
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("traceparent")).toMatch(
      /^00-[\da-f]{32}-[\da-f]{16}-01$/,
    );
    expect(headers.get("x-request-id")).toMatch(/^[\da-f-]{36}$/);
  });

  it("pulls with the client ID and cursor in the query", async () => {
    const body = { cursor: "9", lastMutationId: 0, reset: false, patches: [] };
    const fetchMock = respond(200, body);
    const transport = httpTransport({
      baseUrl: "https://northstar.dev",
      fetch: fetchMock,
    });
    expect(await transport.pull("c 1", "8")).toEqual(body);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "https://northstar.dev/api/v1/sync/pull?clientId=c+1&cursor=8",
    );
  });

  it.each([
    [503, true],
    [429, true],
    [408, true],
    [403, false],
    [400, false],
  ])(
    "maps HTTP %i to a SyncError (retryable: %s)",
    async (status, retryable) => {
      const transport = httpTransport({
        baseUrl: "",
        fetch: respond(status, {}),
      });
      const error = await transport
        .pull("c", "")
        .catch((error_: unknown) => error_);
      expect(error).toBeInstanceOf(SyncError);
      expect(error).toMatchObject({ status, retryable });
    },
  );

  it("treats network failures as retryable", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      Promise.reject(new TypeError("offline")),
    );
    const transport = httpTransport({ baseUrl: "", fetch: fetchMock });
    await expect(
      transport.push({ clientId: "c", mutations: [] }),
    ).rejects.toMatchObject({
      status: 0,
      retryable: true,
    });
  });

  it("uses the global fetch by default", async () => {
    const fetchMock = respond(200, { lastMutationId: 0, rejected: [] });
    vi.stubGlobal("fetch", fetchMock);
    await httpTransport({ baseUrl: "" }).push({ clientId: "c", mutations: [] });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    vi.unstubAllGlobals();
  });
});

describe("response checks", () => {
  it.each([
    NULL,
    [],
    {},
    { lastMutationId: "1", rejected: [] },
    { lastMutationId: 1 },
  ])("rejects the push body %j", (body) => {
    expect(() => parsePush(body)).toThrow(SyncError);
  });

  it.each([
    NULL,
    { cursor: 1, lastMutationId: 0, reset: false, patches: [] },
    { cursor: "1", lastMutationId: 0, reset: "no", patches: [] },
    { cursor: "1", lastMutationId: 0, reset: false, patches: [], more: "yes" },
    { cursor: "1", lastMutationId: 0, reset: false, patches: [{ op: "move" }] },
    {
      cursor: "1",
      lastMutationId: 0,
      reset: false,
      patches: [{ op: "put", collection: 1, key: "k" }],
    },
    { cursor: "1", lastMutationId: 0, reset: false, patches: ["x"] },
  ])("rejects the pull body %j", (body) => {
    expect(() => parsePull(body)).toThrow("malformed pull");
  });

  it("accepts a well-formed pull", () => {
    const body = {
      cursor: "1",
      lastMutationId: 0,
      reset: false,
      patches: [{ op: "del", collection: "issues", key: "ENG-1" }],
    };
    expect(parsePull(body)).toBe(body);
  });
});
