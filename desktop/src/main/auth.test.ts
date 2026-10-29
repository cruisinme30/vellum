// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createHash } from "node:crypto";

import { describe, expect, it, vi } from "vitest";

import { memoryDisk } from "../test-support/memory-disk.js";

import { CLIENT_ID, Session, TokenError, createPkce, discover } from "./auth.js";
import type { Fetch } from "./fetch.js";
import type { Redirect } from "./loopback.js";
import { TokenStore, type Vault } from "./token-store.js";

const server = "https://vellum.northstar.dev";

const vault: Vault = {
  available: () => true,
  backend: () => "os",
  encrypt: (plain) => new TextEncoder().encode(plain),
  decrypt: (sealed) => new TextDecoder().decode(sealed),
};

interface Call {
  readonly url: string;
  readonly body: URLSearchParams;
}

/** A fake authorization server; `tokenStatus` and `metadata` steer it. */
function fakeServer(
  options: {
    metadata?: Record<string, unknown> | undefined;
    tokenStatus?: number;
    userinfo?: unknown;
  } = {},
) {
  const calls: Call[] = [];
  let issued = 0;
  const fetcher: Fetch = (url, init) => {
    calls.push({ url, body: new URLSearchParams(typeof init?.body === "string" ? init.body : "") });
    if (url.endsWith("/.well-known/oauth-authorization-server")) {
      return Promise.resolve(
        options.metadata === undefined
          ? new Response(undefined, { status: 404 })
          : Response.json(options.metadata),
      );
    }
    if (url.endsWith("/token")) {
      issued += 1;
      if (options.tokenStatus !== undefined)
        return Promise.resolve(new Response(undefined, { status: options.tokenStatus }));
      return Promise.resolve(
        Response.json({
          access_token: `at${String(issued)}`,
          refresh_token: `rt${String(issued)}`,
          expires_in: 600,
        }),
      );
    }
    if (url.endsWith("/userinfo")) {
      return Promise.resolve(
        options.userinfo === undefined
          ? new Response(undefined, { status: 500 })
          : Response.json(options.userinfo),
      );
    }
    return Promise.resolve(new Response(undefined, { status: 200 }));
  };
  return { fetcher, calls };
}

function redirect(code = "code1", close: () => void = vi.fn<() => void>()): Redirect {
  return { uri: "http://127.0.0.1:5555/callback", code: Promise.resolve(code), close };
}

const noToken: Fetch = (url) =>
  Promise.resolve(
    url.endsWith("/token")
      ? Response.json({ token_type: "Bearer" })
      : new Response(undefined, { status: 404 }),
  );

const offline: Fetch = () => Promise.reject(new TypeError("offline"));

const fullMetadata = {
  authorization_endpoint: `${server}/oauth/authorize`,
  token_endpoint: `${server}/oauth/token`,
  revocation_endpoint: `${server}/oauth/revoke`,
  userinfo_endpoint: `${server}/oauth/userinfo`,
};

describe("auth", () => {
  it("derives an S256 challenge from the verifier", () => {
    const { verifier, challenge } = createPkce((size) => new Uint8Array(size).fill(7));
    expect(verifier).toHaveLength(43);
    expect(challenge).toBe(createHash("sha256").update(verifier).digest("base64url"));
  });

  it("ignores endpoints on other origins", async () => {
    const { fetcher } = fakeServer({
      metadata: { token_endpoint: "https://evil.example/token", authorization_endpoint: 5 },
    });
    const endpoints = await discover(server, fetcher);
    expect(endpoints.token).toBe(`${server}/oauth/token`);
    expect(endpoints.authorization).toBe(`${server}/oauth/authorize`);
    const bad = fakeServer({ metadata: { token_endpoint: "::" } });
    const fallback = await discover(server, bad.fetcher);
    expect(fallback.token).toBe(`${server}/oauth/token`);
  });

  it("signs in through the system browser with PKCE", async () => {
    const { fetcher, calls } = fakeServer({
      metadata: fullMetadata,
      userinfo: { email: "ana@northstar.dev" },
    });
    const store = new TokenStore(vault, memoryDisk());
    const opened: string[] = [];
    const close = vi.fn<() => void>();
    const loopback = redirect("code1", close);
    const session = new Session(server, store, {
      fetch: fetcher,
      openBrowser: (url) => {
        opened.push(url);
        return Promise.resolve();
      },
      now: () => 0,
      listen: () => Promise.resolve(loopback),
    });
    expect(await session.signIn()).toEqual({ account: "ana@northstar.dev" });
    const authorize = new URL(opened[0] ?? "");
    expect(authorize.searchParams.get("client_id")).toBe(CLIENT_ID);
    expect(authorize.searchParams.get("code_challenge_method")).toBe("S256");
    expect(authorize.searchParams.get("redirect_uri")).toBe(loopback.uri);
    const exchange = calls.find((call) => call.url.endsWith("/token"));
    expect(exchange?.body.get("grant_type")).toBe("authorization_code");
    expect(exchange?.body.get("code")).toBe("code1");
    expect(
      createHash("sha256")
        .update(exchange?.body.get("code_verifier") ?? "")
        .digest("base64url"),
    ).toBe(authorize.searchParams.get("code_challenge"));
    expect(close).toHaveBeenCalled();
    expect(await session.accessToken()).toBe("at1");
  });

  it("names the account after the host without userinfo", async () => {
    for (const userinfo of [undefined, { sub: 5 }]) {
      const { fetcher } = fakeServer({
        metadata: userinfo === undefined ? undefined : fullMetadata,
        userinfo,
      });
      const session = new Session(server, new TokenStore(vault, memoryDisk()), {
        fetch: fetcher,
        openBrowser: () => Promise.resolve(),
        now: () => 0,
        listen: () => Promise.resolve(redirect()),
      });
      expect(await session.signIn()).toEqual({ account: "vellum.northstar.dev" });
    }
    const failing = fakeServer({ metadata: fullMetadata });
    const session = new Session(server, new TokenStore(vault, memoryDisk()), {
      fetch: failing.fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
      listen: () => Promise.resolve(redirect()),
    });
    expect(await session.signIn()).toEqual({ account: "vellum.northstar.dev" });
  });

  it("fails sign-in when the token endpoint refuses", async () => {
    for (const fake of [fakeServer({ tokenStatus: 400 })]) {
      const session = new Session(server, new TokenStore(vault, memoryDisk()), {
        fetch: fake.fetcher,
        openBrowser: () => Promise.resolve(),
        now: () => 0,
        listen: () => Promise.resolve(redirect()),
      });
      await expect(session.signIn()).rejects.toBeInstanceOf(TokenError);
    }
    const session = new Session(server, new TokenStore(vault, memoryDisk()), {
      fetch: noToken,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
      listen: () => Promise.resolve(redirect()),
    });
    await expect(session.signIn()).rejects.toThrow(/token endpoint/);
  });

  it("refreshes once for concurrent callers and keeps the old refresh token if none is returned", async () => {
    const store = new TokenStore(vault, memoryDisk());
    await store.save({
      server,
      account: "ana",
      accessToken: "old",
      refreshToken: "rt0",
      expiresAt: 30_000,
    });
    const { fetcher, calls } = fakeServer();
    const session = new Session(server, store, {
      fetch: fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    });
    expect(await Promise.all([session.accessToken(), session.accessToken()])).toEqual([
      "at1",
      "at1",
    ]);
    expect(calls.filter((call) => call.url.endsWith("/token"))).toHaveLength(1);
    const refreshed = await store.load();
    expect(refreshed?.account).toBe("ana");

    await store.save({
      server,
      account: "ana",
      accessToken: "old",
      refreshToken: "rt0",
      expiresAt: 0,
    });
    const bare: Fetch = (url) =>
      Promise.resolve(
        url.endsWith("/token")
          ? Response.json({ access_token: "fresh" })
          : new Response(undefined, { status: 404 }),
      );
    const plain = new Session(server, store, {
      fetch: bare,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    });
    expect(await plain.accessToken()).toBe("fresh");
    const kept = await store.load();
    expect(kept?.refreshToken).toBe("rt0");
  });

  it("signs out when the refresh grant is refused, and surfaces other failures", async () => {
    const store = new TokenStore(vault, memoryDisk());
    await store.save({
      server,
      account: "ana",
      accessToken: "old",
      refreshToken: "rt0",
      expiresAt: 0,
    });
    const refused = new Session(server, store, {
      fetch: fakeServer({ tokenStatus: 400 }).fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    });
    expect(await refused.accessToken()).toBeUndefined();
    expect(await store.load()).toBeUndefined();

    await store.save({
      server,
      account: "ana",
      accessToken: "old",
      refreshToken: "rt0",
      expiresAt: 0,
    });
    const down = new Session(server, store, {
      fetch: fakeServer({ tokenStatus: 503 }).fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    });
    await expect(down.accessToken()).rejects.toBeInstanceOf(TokenError);

    await store.save({
      server,
      account: "ana",
      accessToken: "old",
      refreshToken: undefined,
      expiresAt: 0,
    });
    expect(await down.accessToken()).toBeUndefined();
    expect(await store.load()).toBeUndefined();
  });

  it("ignores tokens for another server", async () => {
    const store = new TokenStore(vault, memoryDisk());
    await store.save({
      server: "https://other.dev",
      account: "ana",
      accessToken: "x",
      refreshToken: undefined,
      expiresAt: 9e15,
    });
    const session = new Session(server, store, {
      fetch: fakeServer().fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    });
    expect(await session.accessToken()).toBeUndefined();
  });

  it("revokes the refresh token on sign-out when it can", async () => {
    const store = new TokenStore(vault, memoryDisk());
    const revoking = fakeServer({ metadata: fullMetadata });
    await store.save({
      server,
      account: "ana",
      accessToken: "at",
      refreshToken: "rt9",
      expiresAt: 9e15,
    });
    await new Session(server, store, {
      fetch: revoking.fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    }).signOut();
    expect(revoking.calls.find((call) => call.url.endsWith("/revoke"))?.body.get("token")).toBe(
      "rt9",
    );
    expect(await store.load()).toBeUndefined();

    const silent = fakeServer({ metadata: { token_endpoint: `${server}/oauth/token` } });
    await store.save({
      server,
      account: "ana",
      accessToken: "at",
      refreshToken: "rt9",
      expiresAt: 9e15,
    });
    await new Session(server, store, {
      fetch: silent.fetcher,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    }).signOut();
    expect(silent.calls.some((call) => call.url.endsWith("/revoke"))).toBe(false);

    await store.save({
      server,
      account: "ana",
      accessToken: "at",
      refreshToken: "rt9",
      expiresAt: 9e15,
    });
    await new Session(server, store, {
      fetch: offline,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    }).signOut();
    expect(await store.load()).toBeUndefined();

    await new Session(server, store, {
      fetch: offline,
      openBrowser: () => Promise.resolve(),
      now: () => 0,
    }).signOut();
  });
});
