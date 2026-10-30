// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createHash, randomBytes } from "node:crypto";

import type { Fetch } from "./fetch.js";
import { waitForRedirect, type Redirect } from "./loopback.js";
import type { Tokens, TokenStore } from "./token-store.js";

/**
 * Sign-in through the system browser (ADR 0023, D99): OAuth 2.1 with PKCE
 * and a loopback redirect (RFC 8252) as the public client `vellum-desktop`.
 * The app never sees the password and works with SSO, passkeys and MFA.
 */

export const CLIENT_ID = "vellum-desktop";
const SCOPES = "read write:direct work";
/** Refresh this long before the access token expires. */
const REFRESH_MARGIN_MS = 60_000;

interface Endpoints {
  readonly authorization: string;
  readonly token: string;
  readonly revocation: string | undefined;
  readonly userinfo: string;
}

export interface AuthDeps {
  readonly fetch: Fetch;
  readonly openBrowser: (url: string) => Promise<void>;
  readonly now: () => number;
  /** Starts the loopback listener; tests replace it. */
  readonly listen?: (state: string) => Promise<Redirect>;
}

/** A PKCE verifier and its S256 challenge. */
export function createPkce(random: (size: number) => Uint8Array = randomBytes) {
  const verifier = base64Url(random(32));
  const challenge = base64Url(createHash("sha256").update(verifier).digest());
  return { verifier, challenge };
}

/** Reads RFC 8414 metadata, falling back to Vellum's default paths. */
export async function discover(
  server: string,
  fetcher: Fetch,
): Promise<Endpoints> {
  const fallback: Endpoints = {
    authorization: `${server}/oauth/authorize`,
    token: `${server}/oauth/token`,
    revocation: `${server}/oauth/revoke`,
    userinfo: `${server}/oauth/userinfo`,
  };
  const response = await fetcher(
    `${server}/.well-known/oauth-authorization-server`,
    {
      headers: { accept: "application/json" },
    },
  );
  if (!response.ok) return fallback;
  const metadata = (await response.json()) as Record<string, unknown>;
  return {
    authorization:
      sameOrigin(server, metadata["authorization_endpoint"]) ??
      fallback.authorization,
    token: sameOrigin(server, metadata["token_endpoint"]) ?? fallback.token,
    revocation: sameOrigin(server, metadata["revocation_endpoint"]),
    userinfo:
      sameOrigin(server, metadata["userinfo_endpoint"]) ?? fallback.userinfo,
  };
}

/** The signed-in session for one server, with single-flight refresh. */
export class Session {
  private refreshing: Promise<Tokens | undefined> | undefined;

  constructor(
    private readonly server: string,
    private readonly store: TokenStore,
    private readonly deps: AuthDeps,
  ) {}

  /** Opens the system browser and waits for the redirect back. */
  async signIn(): Promise<{ account: string }> {
    const endpoints = await discover(this.server, this.deps.fetch);
    const { verifier, challenge } = createPkce();
    const state = base64Url(randomBytes(16));
    const redirect = await (this.deps.listen ?? waitForRedirect)(state);
    try {
      const url = new URL(endpoints.authorization);
      url.search = new URLSearchParams({
        response_type: "code",
        client_id: CLIENT_ID,
        redirect_uri: redirect.uri,
        scope: SCOPES,
        state,
        code_challenge: challenge,
        code_challenge_method: "S256",
      }).toString();
      await this.deps.openBrowser(url.toString());
      const code = await redirect.code;
      const tokens = await this.token(endpoints, {
        grant_type: "authorization_code",
        code,
        redirect_uri: redirect.uri,
        code_verifier: verifier,
      });
      const account = await this.account(endpoints, tokens.accessToken);
      await this.store.save({ ...tokens, account });
      return { account };
    } finally {
      redirect.close();
    }
  }

  /**
   * Revokes the refresh token when the server supports it, then forgets both
   * tokens.
   */
  async signOut(): Promise<void> {
    const tokens = await this.store.load();
    await this.store.clear();
    if (tokens?.refreshToken === undefined) return;
    try {
      const endpoints = await discover(this.server, this.deps.fetch);
      if (endpoints.revocation === undefined) return;
      await this.deps.fetch(
        endpoints.revocation,
        form({ token: tokens.refreshToken, client_id: CLIENT_ID }),
      );
    } catch {
      // Signing out locally must not depend on the server being reachable.
    }
  }

  /**
   * A valid access token, refreshed if it is about to expire; undefined when
   * signed out.
   */
  async accessToken(): Promise<string | undefined> {
    const tokens = await this.store.load();
    if (tokens?.server !== this.server) return undefined;
    if (tokens.expiresAt - REFRESH_MARGIN_MS > this.deps.now())
      return tokens.accessToken;
    this.refreshing ??= this.refresh(tokens).finally(() => {
      this.refreshing = undefined;
    });
    const refreshed = await this.refreshing;
    return refreshed?.accessToken;
  }

  private async refresh(tokens: Tokens): Promise<Tokens | undefined> {
    if (tokens.refreshToken === undefined) {
      await this.store.clear();
      return undefined;
    }
    const endpoints = await discover(this.server, this.deps.fetch);
    try {
      const fresh = await this.token(endpoints, {
        grant_type: "refresh_token",
        refresh_token: tokens.refreshToken,
      });
      const next = {
        ...fresh,
        refreshToken: fresh.refreshToken ?? tokens.refreshToken,
        account: tokens.account,
      };
      await this.store.save(next);
      return next;
    } catch (error) {
      if (error instanceof TokenError && error.status === 400) {
        // invalid_grant: the session was revoked or expired on the server.
        await this.store.clear();
        return undefined;
      }
      throw error;
    }
  }

  private async token(
    endpoints: Endpoints,
    body: Record<string, string>,
  ): Promise<Tokens> {
    const response = await this.deps.fetch(
      endpoints.token,
      form({ ...body, client_id: CLIENT_ID }),
    );
    if (!response.ok) throw new TokenError(response.status);
    const value = (await response.json()) as Record<string, unknown>;
    const accessToken = value["access_token"];
    const expiresIn = value["expires_in"];
    if (typeof accessToken !== "string") throw new TokenError(response.status);
    const refreshToken = value["refresh_token"];
    return {
      server: this.server,
      account: "",
      accessToken,
      refreshToken: typeof refreshToken === "string" ? refreshToken : undefined,
      expiresAt:
        this.deps.now() +
        (typeof expiresIn === "number" ? expiresIn : 3600) * 1000,
    };
  }

  private async account(
    endpoints: Endpoints,
    accessToken: string,
  ): Promise<string> {
    const host = new URL(this.server).host;
    const response = await this.deps.fetch(endpoints.userinfo, {
      headers: {
        authorization: `Bearer ${accessToken}`,
        accept: "application/json",
      },
    });
    if (!response.ok) return host;
    const info = (await response.json()) as Record<string, unknown>;
    const name = info["email"] ?? info["preferred_username"] ?? info["sub"];
    return typeof name === "string" ? name : host;
  }
}

/** The token endpoint refused a grant. */
export class TokenError extends Error {
  constructor(readonly status: number) {
    super(`The token endpoint answered ${String(status)}`);
    this.name = "TokenError";
  }
}

function form(body: Record<string, string>): RequestInit {
  return {
    method: "POST",
    headers: {
      "content-type": "application/x-www-form-urlencoded",
      accept: "application/json",
    },
    body: new URLSearchParams(body).toString(),
  };
}

function sameOrigin(server: string, value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  try {
    return new URL(value).origin === new URL(server).origin ? value : undefined;
  } catch {
    return undefined;
  }
}

function base64Url(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}
