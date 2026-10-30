// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The signed-in person's own account (`/api/v1/me`): personal API tokens
 * (issue #523, ADR 0069). Writes go through the app's HTTP client, so
 * creating a token after an old sign-in asks the person to confirm who
 * they are first (D241).
 */

import type { CreatedToken, Scope, TokenPolicy, TokenView } from "../api/api.gen";
import type { Http } from "../api/http";

/** A new token: a label, the scopes and the days until it expires. */
interface NewToken {
  readonly name: string;
  readonly scopes: readonly Scope[];
  /** Absent for a token that never expires, when the workspace allows it. */
  readonly expiresInDays?: number;
}

/** The account operations the settings screens use. */
export interface AccountApi {
  /** The caller's tokens, newest first; never their secrets. */
  tokens(): Promise<TokenView[]>;
  /** Which scopes the caller may grant, and the longest expiry. */
  tokenPolicy(): Promise<TokenPolicy>;
  /** Creates a token; the secret in the answer is shown only once. */
  createToken(input: NewToken): Promise<CreatedToken>;
  revokeToken(id: string): Promise<void>;
}

const ME = "/api/v1/me";

/** The account API over a Vellum server. */
export function createHttpAccountApi(http: Http): AccountApi {
  return {
    tokens: () => http.get<TokenView[]>(`${ME}/tokens`),
    tokenPolicy: () => http.get<TokenPolicy>(`${ME}/token-policy`),
    createToken: (input) => http.send<CreatedToken>("POST", `${ME}/tokens`, input),
    async revokeToken(id) {
      await http.send("DELETE", `${ME}/tokens/${encodeURIComponent(id)}`);
    },
  };
}

export { type CreatedToken, type Scope, type TokenPolicy, type TokenView } from "../api/api.gen";
