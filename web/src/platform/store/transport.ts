// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { newRequestId, newTraceparent } from "../telemetry";

import type { PullResponse, PushRequest, PushResponse } from "./types";

/** How the store talks to the server. Tests and the desktop shell swap it. */
export interface SyncTransport {
  push(request: PushRequest): Promise<PushResponse>;
  pull(clientId: string, cursor: string): Promise<PullResponse>;
}

/** A failed sync call. */
export class SyncError extends Error {
  /**
   * @param status - HTTP status, or 0 for a network failure
   * @param retryable - whether trying again later can succeed
   * @param requestId - the `X-Request-Id` sent, for the error dialog
   */
  constructor(
    message: string,
    readonly status: number,
    readonly retryable: boolean,
    readonly requestId: string,
  ) {
    super(message);
  }
}

/** Options for {@link httpTransport}. */
export interface HttpTransportOptions {
  /** Server origin plus any path prefix, without a trailing slash. */
  readonly baseUrl: string;
  /** Defaults to the global `fetch`. */
  readonly fetch?: typeof fetch;
}

/**
 * The HTTP transport: `POST {base}/api/v1/sync/push` and
 * `GET {base}/api/v1/sync/pull`. Authentication is the session cookie on
 * the web and a bearer token added by the desktop bridge.
 */
export function httpTransport(options: HttpTransportOptions): SyncTransport {
  const doFetch = options.fetch ?? globalThis.fetch.bind(globalThis);
  const call = async (path: string, init: RequestInit): Promise<unknown> => {
    const requestId = newRequestId();
    const headers = new Headers(init.headers);
    headers.set("accept", "application/json");
    headers.set("x-request-id", requestId);
    headers.set("traceparent", newTraceparent());
    let response: Response;
    try {
      response = await doFetch(`${options.baseUrl}${path}`, {
        ...init,
        headers,
        credentials: "include",
      });
    } catch (error) {
      throw new SyncError(`network: ${String(error)}`, 0, true, requestId);
    }
    if (!response.ok) {
      const retryable =
        response.status === 408 ||
        response.status === 429 ||
        response.status >= 500;
      throw new SyncError(
        `HTTP ${String(response.status)}`,
        response.status,
        retryable,
        requestId,
      );
    }
    return response.json() as Promise<unknown>;
  };
  return {
    async push(request) {
      const body = await call("/api/v1/sync/push", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request),
      });
      return parsePush(body);
    },
    async pull(clientId, cursor) {
      const query = new URLSearchParams({ clientId, cursor });
      return parsePull(
        await call(`/api/v1/sync/pull?${query.toString()}`, { method: "GET" }),
      );
    },
  };
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function malformed(what: string): never {
  throw new SyncError(`malformed ${what} response`, 200, false, "");
}

/** Checks a push response from the network. */
export function parsePush(body: unknown): PushResponse {
  if (
    !isObject(body) ||
    typeof body["lastMutationId"] !== "number" ||
    !Array.isArray(body["rejected"])
  ) {
    return malformed("push");
  }
  return body as unknown as PushResponse;
}

function isPatch(patch: unknown): boolean {
  return (
    isObject(patch) &&
    (patch["op"] === "put" || patch["op"] === "del") &&
    typeof patch["collection"] === "string" &&
    typeof patch["key"] === "string"
  );
}

function isPullShape(body: Record<string, unknown>): boolean {
  return (
    typeof body["cursor"] === "string" &&
    typeof body["lastMutationId"] === "number" &&
    typeof body["reset"] === "boolean" &&
    (body["more"] === undefined || typeof body["more"] === "boolean") &&
    Array.isArray(body["patches"])
  );
}

/** Checks a pull response from the network. */
export function parsePull(body: unknown): PullResponse {
  if (
    !isObject(body) ||
    !isPullShape(body) ||
    !(body["patches"] as unknown[]).every((patch) => isPatch(patch))
  ) {
    return malformed("pull");
  }
  return body as unknown as PullResponse;
}
