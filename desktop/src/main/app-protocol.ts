// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import path from "node:path";

import type { Fetch } from "./fetch.js";

/**
 * Serves `app://vellum/`: the built web app from disk, and `/api/*`
 * forwarded to the Vellum server with the access token added here in the
 * main process. Pages stay same-origin and never handle cookies.
 */

export interface ProtocolOptions {
  /** Folder holding the built web app (`web/dist`). */
  readonly webRoot: string;
  readonly server: () => string;
  readonly accessToken: () => Promise<string | undefined>;
  readonly readFile: (file: string) => Promise<Uint8Array | undefined>;
  readonly fetch: Fetch;
  readonly csp: () => string;
}

const contentTypes: Readonly<Record<string, string>> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
  ".txt": "text/plain; charset=utf-8",
};

/** Request headers the page may not set on proxied calls. */
const strippedHeaders = ["authorization", "cookie", "host", "origin", "referer"];

/**
 * Maps a URL path to a file under `webRoot`, or undefined if it would
 * escape it. Paths without an extension are app routes and get `index.html`.
 */
export function resolveAsset(webRoot: string, pathname: string): string | undefined {
  let decoded: string;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return undefined;
  }
  if (decoded.includes("\0") || decoded.includes("\\")) return undefined;
  const relative = path.posix.normalize(decoded).replace(/^\/+/, "");
  if (relative.startsWith("..")) return undefined;
  const file = relative === "" || path.posix.extname(relative) === "" ? "index.html" : relative;
  const root = path.resolve(webRoot);
  const resolved = path.resolve(root, file);
  return resolved.startsWith(root + path.sep) ? resolved : undefined;
}

/** Answers one `app://` request. */
export async function handleAppRequest(
  request: Request,
  options: ProtocolOptions,
): Promise<Response> {
  const url = new URL(request.url);
  if (url.pathname.startsWith("/api/")) return proxy(request, url, options);
  if (request.method !== "GET" && request.method !== "HEAD")
    return new Response(undefined, { status: 405 });
  const file = resolveAsset(options.webRoot, url.pathname);
  if (file === undefined) return new Response(undefined, { status: 404 });
  const body = await options.readFile(file);
  if (body === undefined) return new Response(undefined, { status: 404 });
  const extension = path.extname(file);
  const headers = new Headers({
    "content-type": contentTypes[extension] ?? "application/octet-stream",
    "x-content-type-options": "nosniff",
  });
  if (extension === ".html") {
    headers.set("content-security-policy", options.csp());
    headers.set("cache-control", "no-cache");
  }
  return new Response(request.method === "HEAD" ? undefined : new Uint8Array(body), { headers });
}

async function proxy(request: Request, url: URL, options: ProtocolOptions): Promise<Response> {
  const target = new URL(`${url.pathname}${url.search}`, options.server());
  const headers = new Headers(request.headers);
  for (const name of strippedHeaders) headers.delete(name);
  const token = await options.accessToken();
  if (token !== undefined) headers.set("authorization", `Bearer ${token}`);
  const hasBody = request.method !== "GET" && request.method !== "HEAD";
  try {
    const response = await options.fetch(target.href, {
      method: request.method,
      headers,
      redirect: "manual",
      ...(hasBody ? { body: await request.arrayBuffer() } : {}),
    });
    const out = new Headers(response.headers);
    out.delete("set-cookie");
    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers: out,
    });
  } catch {
    // Offline or the server is down: the sync layer treats 503 as retryable.
    return Response.json(
      { error: { code: "unavailable", message: "The Vellum server cannot be reached" } },
      { status: 503 },
    );
  }
}
