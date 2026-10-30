// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import path from "node:path";

import { describe, expect, it } from "vitest";

import {
  handleAppRequest,
  resolveAsset,
  type ProtocolOptions,
} from "./app-protocol.js";
import type { Fetch } from "./fetch.js";

const root = path.resolve("/srv/web");

function options(overrides: Partial<ProtocolOptions> = {}): ProtocolOptions {
  const files = new Map<string, string>([
    [path.join(root, "index.html"), "<!doctype html>"],
    [path.join(root, "assets", "app.js"), "console.log(1)"],
    [path.join(root, "data.bin"), "x"],
  ]);
  return {
    webRoot: root,
    server: () => "https://vellum.northstar.dev",
    accessToken: () => Promise.resolve("token1"),
    readFile: (file) => {
      const text = files.get(file);
      return Promise.resolve(
        text === undefined ? undefined : new TextEncoder().encode(text),
      );
    },
    fetch: () => Promise.resolve(new Response("{}")),
    csp: () => "default-src 'self'",
    ...overrides,
  };
}

describe("resolveAsset", () => {
  it("serves files and falls back to index.html for app routes", () => {
    expect(resolveAsset(root, "/")).toBe(path.join(root, "index.html"));
    expect(resolveAsset(root, "/p/abc")).toBe(path.join(root, "index.html"));
    expect(resolveAsset(root, "/assets/app.js")).toBe(
      path.join(root, "assets", "app.js"),
    );
  });

  it("never leaves the web root", () => {
    expect(resolveAsset(root, "/../secret.txt")).toBe(
      path.join(root, "secret.txt"),
    );
    expect(resolveAsset(root, "/%2e%2e/%2e%2e/etc/passwd.txt")).toBe(
      path.join(root, "etc", "passwd.txt"),
    );
    expect(resolveAsset(root, "/a%5c..%5cb.js")).toBeUndefined();
    expect(resolveAsset(root, "/a%00.js")).toBeUndefined();
    expect(resolveAsset(root, "/%E0%A4%A")).toBeUndefined();
  });
});

/** A token store with nobody signed in. */
async function signedOut(): Promise<string | undefined> {
  await Promise.resolve();
  return process.env["VELLUM_TEST_ACCESS_TOKEN"];
}

async function status(url: string, method = "GET"): Promise<number> {
  const response = await handleAppRequest(
    new Request(url, { method }),
    options(),
  );
  return response.status;
}

describe("handleAppRequest", () => {
  it("serves the app with a CSP on HTML", async () => {
    const page = await handleAppRequest(
      new Request("app://vellum/p/1"),
      options(),
    );
    expect(page.headers.get("content-type")).toBe("text/html; charset=utf-8");
    expect(page.headers.get("content-security-policy")).toBe(
      "default-src 'self'",
    );
    expect(await page.text()).toBe("<!doctype html>");

    const script = await handleAppRequest(
      new Request("app://vellum/assets/app.js"),
      options(),
    );
    expect(script.headers.get("content-type")).toBe(
      "text/javascript; charset=utf-8",
    );
    expect(script.headers.get("content-security-policy")).toBeNull();

    const binary = await handleAppRequest(
      new Request("app://vellum/data.bin"),
      options(),
    );
    expect(binary.headers.get("content-type")).toBe("application/octet-stream");

    const head = await handleAppRequest(
      new Request("app://vellum/", { method: "HEAD" }),
      options(),
    );
    expect(head.status).toBe(200);
    expect(await head.text()).toBe("");
  });

  it("answers 404 and 405 where it should", async () => {
    expect(await status("app://vellum/missing.js")).toBe(404);
    expect(await status("app://vellum/a%00.js")).toBe(404);
    expect(await status("app://vellum/", "POST")).toBe(405);
  });

  it("forwards API calls with the bearer token and without page credentials", async () => {
    let seen: { url: string; init: RequestInit | undefined } | undefined;
    const fetcher: Fetch = (url, init) => {
      seen = { url, init };
      return Promise.resolve(
        new Response('{"ok":true}', {
          status: 201,
          headers: { "set-cookie": "a=b", "content-type": "application/json" },
        }),
      );
    };
    const response = await handleAppRequest(
      new Request("app://vellum/api/v1/sync/push?x=1", {
        method: "POST",
        headers: {
          authorization: "Bearer stolen",
          cookie: "c=d",
          "x-request-id": "r1",
        },
        body: '{"m":1}',
      }),
      options({ fetch: fetcher }),
    );
    expect(response.status).toBe(201);
    expect(response.headers.get("set-cookie")).toBeNull();
    expect(seen?.url).toBe("https://vellum.northstar.dev/api/v1/sync/push?x=1");
    const headers = new Headers(seen?.init?.headers);
    expect(headers.get("authorization")).toBe("Bearer token1");
    expect(headers.get("cookie")).toBeNull();
    expect(headers.get("x-request-id")).toBe("r1");
    expect(new TextDecoder().decode(seen?.init?.body as ArrayBuffer)).toBe(
      '{"m":1}',
    );
    expect(seen?.init?.redirect).toBe("manual");
  });

  it(
    "forwards signed-out GETs without a token and reports an unreachable " +
      "server",
    async () => {
      let headers: Headers | undefined;
      const fetcher: Fetch = (_url, init) => {
        headers = new Headers(init?.headers);
        expect(init?.body).toBeUndefined();
        return Promise.resolve(new Response("[]"));
      };
      await handleAppRequest(
        new Request("app://vellum/api/v1/me"),
        options({ fetch: fetcher, accessToken: signedOut }),
      );
      expect(headers?.get("authorization")).toBeNull();

      const offline = await handleAppRequest(
        new Request("app://vellum/api/v1/me"),
        options({ fetch: () => Promise.reject(new TypeError("offline")) }),
      );
      expect(offline.status).toBe(503);
    },
  );
});
