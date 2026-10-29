// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The one HTTP client the app uses to reach the server (issue #392).
 *
 * - Requests go to the same origin with the session cookie.
 * - Unsafe requests (POST, PUT, PATCH, DELETE) carry the CSRF token from
 *   `GET /api/csrf` and a fresh `Idempotency-Key`, so a retry never applies
 *   a write twice.
 * - Error responses (`ApiError` on the server: `code`, `message`,
 *   `retryable`, `request_id`) become an {@link ApiError} that views can
 *   show as is.
 */

/** The server's stable error codes, plus `network` when no answer came. */
export type ErrorCode =
  | "invalid_input"
  | "unauthenticated"
  | "permission_denied"
  | "not_found"
  | "conflict"
  | "rate_limited"
  | "unavailable"
  | "timeout"
  | "internal"
  | "network";

const CODES = new Set<string>([
  "invalid_input",
  "unauthenticated",
  "permission_denied",
  "not_found",
  "conflict",
  "rate_limited",
  "unavailable",
  "timeout",
  "internal",
]);

/** A failed request. `message` is safe to show; quote `requestId` when reporting it. */
export class ApiError extends Error {
  readonly code: ErrorCode;
  readonly status: number;
  readonly retryable: boolean;
  readonly requestId: string | undefined;

  constructor(
    code: ErrorCode,
    message: string,
    details: { status?: number; retryable?: boolean; requestId?: string } = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = details.status ?? 0;
    this.retryable = details.retryable ?? false;
    this.requestId = details.requestId;
  }
}

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

/** Query parameters; a list is sent as the same key repeated. */
export type Query = Readonly<
  Record<string, string | number | boolean | readonly string[] | undefined>
>;

export interface Http {
  get<T>(path: string, query?: Query): Promise<T>;
  /** An unsafe request with a JSON body. */
  send<T>(method: Exclude<Method, "GET">, path: string, body?: unknown): Promise<T>;
  /**
   * A POST whose body is raw bytes (a file upload), with its media type.
   * Like `send` it carries the CSRF token and an idempotency key.
   */
  upload<T>(path: string, query: Query, body: Blob, type: string): Promise<T>;
  /** Forgets the CSRF token; called after signing in or out, when the session changes. */
  forgetSession(): void;
}

export interface HttpOptions {
  /** Prefix for every path; `""` for same-origin absolute paths. */
  readonly base?: string;
  readonly fetch?: typeof fetch;
  /** Makes idempotency keys; tests pass a fixed one. */
  readonly idempotencyKey?: () => string;
}

/** Builds a query string; lists repeat their key, empty values are left out. */
export function queryString(query: Query | undefined): string {
  const parameters = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value === undefined) continue;
    if (typeof value === "object") {
      for (const item of value) parameters.append(key, item);
    } else {
      parameters.append(key, String(value));
    }
  }
  const text = parameters.toString();
  return text === "" ? "" : `?${text}`;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

/** The request ID from the body, else from the `X-Request-Id` header. */
function requestIdOf(fromBody: unknown, header: string | undefined): { requestId?: string } {
  if (typeof fromBody === "string") return { requestId: fromBody };
  return header === undefined ? {} : { requestId: header };
}

async function errorOf(response: Response): Promise<ApiError> {
  const header = response.headers.get("x-request-id") ?? undefined;
  let body: unknown;
  try {
    body = await response.json();
  } catch {
    body = undefined;
  }
  if (isRecord(body) && typeof body["message"] === "string") {
    const code =
      typeof body["code"] === "string" && CODES.has(body["code"]) ? body["code"] : "internal";
    return new ApiError(code as ErrorCode, body["message"], {
      status: response.status,
      retryable: body["retryable"] === true,
      ...requestIdOf(body["request_id"], header),
    });
  }
  return new ApiError(
    response.status >= 500 ? "unavailable" : "internal",
    response.statusText || "The server did not answer as expected.",
    {
      status: response.status,
      retryable: response.status >= 500,
      ...(header === undefined ? {} : { requestId: header }),
    },
  );
}

/** Creates the client. One per app; it caches the CSRF token. */
export function createHttp(options: HttpOptions = {}): Http {
  const base = options.base ?? "";
  const doFetch = options.fetch ?? ((input, init) => fetch(input, init));
  const idempotencyKey = options.idempotencyKey ?? (() => crypto.randomUUID());
  let csrf: Promise<string | undefined> | undefined;

  const request = async (method: Method, url: string, init: RequestInit): Promise<Response> => {
    try {
      return await doFetch(`${base}${url}`, { ...init, method, credentials: "same-origin" });
    } catch {
      throw new ApiError("network", "Vellum could not reach the server. Check your connection.", {
        retryable: true,
      });
    }
  };

  /** The session's CSRF token; none before sign-in, when only the origin is checked. */
  const csrfToken = (): Promise<string | undefined> => {
    csrf ??= (async () => {
      try {
        const response = await request("GET", "/api/csrf", {
          headers: { accept: "application/json" },
        });
        if (!response.ok) return;
        const body: unknown = await response.json();
        return isRecord(body) && typeof body["token"] === "string" ? body["token"] : undefined;
      } catch {
        return;
      }
    })();
    return csrf;
  };

  const parse = async <T>(response: Response): Promise<T> => {
    if (!response.ok) throw await errorOf(response);
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  };

  /** A request that changes something: CSRF token, idempotency key, one retry for a new session. */
  const unsafe = async <T>(
    method: Exclude<Method, "GET">,
    path: string,
    content: { readonly body: BodyInit; readonly type: string } | undefined,
  ): Promise<T> => {
    const key = idempotencyKey();
    const attempt = async (): Promise<Response> => {
      const token = await csrfToken();
      const headers: Record<string, string> = {
        accept: "application/json",
        "idempotency-key": key,
      };
      if (token !== undefined) headers["x-csrf-token"] = token;
      if (content !== undefined) headers["content-type"] = content.type;
      return request(method, path, {
        headers,
        ...(content === undefined ? {} : { body: content.body }),
      });
    };
    let response = await attempt();
    // A new session (just signed in or out) needs a new token: fetch it once more.
    if (response.status === 403) {
      const error = await errorOf(response.clone());
      if (/csrf/i.test(error.message)) {
        csrf = undefined;
        response = await attempt();
      }
    }
    return parse<T>(response);
  };

  return {
    async get<T>(path: string, query?: Query): Promise<T> {
      const response = await request("GET", `${path}${queryString(query)}`, {
        headers: { accept: "application/json" },
      });
      return parse<T>(response);
    },

    send<T>(method: Exclude<Method, "GET">, path: string, body?: unknown): Promise<T> {
      return unsafe<T>(
        method,
        path,
        body === undefined ? undefined : { body: JSON.stringify(body), type: "application/json" },
      );
    },

    upload<T>(path: string, query: Query, body: Blob, type: string): Promise<T> {
      return unsafe<T>("POST", `${path}${queryString(query)}`, { body, type });
    },

    forgetSession(): void {
      csrf = undefined;
    },
  };
}
