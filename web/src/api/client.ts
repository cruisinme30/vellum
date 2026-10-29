// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// A typed client for the Vellum API. The operation table and every input
// and output type come from the server (`api.gen.ts`); this file only turns
// a call into an HTTP request the gateway understands.

import { OPERATIONS, type ApiError, type Operations } from "./api.gen";

/** An operation ID, such as `drive.get`. */
export type OperationId = keyof Operations;

/** What a call needs. */
export type Input<K extends OperationId> = Operations[K]["input"];

/** What a call returns. */
type Output<K extends OperationId> = Operations[K]["output"];

/** A failed call: the gateway's error body, plus the HTTP status. */
export class VellumApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly retryable: boolean;
  readonly requestId: string;

  constructor(status: number, body: ApiError) {
    super(body.message);
    this.name = "VellumApiError";
    this.status = status;
    this.code = body.code;
    this.retryable = body.retryable;
    this.requestId = body.request_id;
  }
}

/** How the client reaches the server. */
export interface ClientOptions {
  /** Where the API lives; `/api` on the app's own origin by default. */
  readonly baseUrl?: string;
  /** A scoped API token or `OAuth` access token; omit to use the session cookie. */
  readonly token?: string;
  /** The fetch to use (tests, service workers). */
  readonly fetch?: typeof fetch;
}

/** Per-call options. */
interface CallOptions {
  /** Makes a retried write safe: the server answers a repeat with the first result. */
  readonly idempotencyKey?: string;
  readonly signal?: AbortSignal;
}

/** A request, before it is sent. */
export interface PreparedRequest {
  readonly method: string;
  /** The path and query string, relative to the base URL. */
  readonly url: string;
  /** The JSON body, for writes. */
  readonly body?: string;
}

function scalar(name: string, value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (value === null) {
    return "null";
  }
  throw new TypeError(`\`${name}\` cannot be sent in a URL`);
}

/**
 * Builds the HTTP request for an operation: path parameters filled from
 * the input, the rest as a query string (reads and deletes) or a JSON body.
 */
export function prepare<K extends OperationId>(id: K, input: Input<K>): PreparedRequest {
  const operation = OPERATIONS[id];
  const fields: Record<string, unknown> = { ...(input as Record<string, unknown>) };
  const inPath = new Set<string>();
  const path = operation.path
    .split("/")
    .map((segment) => {
      if (!segment.startsWith("{") || !segment.endsWith("}")) {
        return segment;
      }
      const name = segment.slice(1, -1);
      const value = fields[name];
      if (value === undefined) {
        throw new TypeError(`${id} needs \`${name}\``);
      }
      inPath.add(name);
      return encodeURIComponent(scalar(name, value));
    })
    .join("/");
  if (operation.method === "GET" || operation.method === "DELETE") {
    const query = new URLSearchParams();
    for (const [name, value] of Object.entries(fields)) {
      if (value === undefined || inPath.has(name)) {
        continue;
      }
      for (const item of Array.isArray(value) ? (value as unknown[]) : [value]) {
        query.append(name, scalar(name, item));
      }
    }
    const search = query.toString();
    return { method: operation.method, url: search ? `${path}?${search}` : path };
  }
  return { method: operation.method, url: path, body: JSON.stringify(fields) };
}

function isApiError(body: unknown): body is ApiError {
  return (
    typeof body === "object" &&
    body !== null &&
    typeof (body as Record<string, unknown>)["code"] === "string" &&
    typeof (body as Record<string, unknown>)["message"] === "string"
  );
}

async function failure(response: Response): Promise<VellumApiError> {
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    // Not JSON: a proxy or load balancer answered.
  }
  if (isApiError(body)) {
    return new VellumApiError(response.status, body);
  }
  return new VellumApiError(response.status, {
    code: "internal",
    message: `the server answered ${String(response.status)}`,
    retryable: response.status >= 500,
    request_id: response.headers.get("x-request-id") ?? "",
  });
}

/** A client bound to one server. */
export interface Client {
  call<K extends OperationId>(id: K, input: Input<K>, options?: CallOptions): Promise<Output<K>>;
}

/** Creates a client. Browser sessions fetch the CSRF token once, on the first write. */
export function createClient(options: ClientOptions = {}): Client {
  const base = (options.baseUrl ?? "/api").replace(/\/$/, "");
  const send = options.fetch ?? globalThis.fetch.bind(globalThis);
  let csrf: Promise<string> | undefined;

  async function csrfToken(): Promise<string> {
    csrf ??= send(`${base}/csrf`, { credentials: "same-origin" }).then(async (response) => {
      if (!response.ok) {
        throw await failure(response);
      }
      const body = (await response.json()) as { token: string };
      return body.token;
    });
    try {
      return await csrf;
    } catch (error) {
      csrf = undefined;
      throw error;
    }
  }

  async function call<K extends OperationId>(
    id: K,
    input: Input<K>,
    callOptions: CallOptions = {},
  ): Promise<Output<K>> {
    const request = prepare(id, input);
    const headers = new Headers({ accept: "application/json" });
    if (request.body !== undefined) {
      headers.set("content-type", "application/json");
    }
    if (options.token === undefined) {
      if (request.method !== "GET") {
        headers.set("x-csrf-token", await csrfToken());
      }
    } else {
      headers.set("authorization", `Bearer ${options.token}`);
    }
    if (callOptions.idempotencyKey !== undefined && OPERATIONS[id].mutation) {
      headers.set("idempotency-key", callOptions.idempotencyKey);
    }
    const init: RequestInit = {
      method: request.method,
      headers,
      credentials: "same-origin",
    };
    if (request.body !== undefined) {
      init.body = request.body;
    }
    if (callOptions.signal !== undefined) {
      init.signal = callOptions.signal;
    }
    const response = await send(`${base}${request.url}`, init);
    if (!response.ok) {
      throw await failure(response);
    }
    return (await response.json()) as Output<K>;
  }

  return { call };
}
