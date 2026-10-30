// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The signed-in person's own account (`/api/v1/me`): personal API tokens
 * and second factors (issue #523, ADR 0069). Writes go through the app's
 * HTTP client, so creating a token or changing a factor after an old
 * sign-in asks the person to confirm who they are first (D241).
 */

import type {
  CreatedToken,
  FactorAdded,
  FactorsView,
  PasskeyCeremony,
  RecoveryCodes,
  Scope,
  TokenPolicy,
  TokenView,
  TotpSetup,
} from "../api/api.gen";
import type { Http } from "../api/http";

import type { CreationOptionsJson, RegistrationJson } from "./webauthn";

/** A new token: a label, the scopes and the days until it expires. */
interface NewToken {
  readonly name: string;
  readonly scopes: readonly Scope[];
  /** Absent for a token that never expires, when the workspace allows it. */
  readonly expiresInDays?: number;
}

/** A passkey ceremony to run: its ticket and the browser's options. */
interface PasskeyStart {
  readonly ticket: string;
  readonly options: CreationOptionsJson;
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
  /** The caller's authenticator app, passkeys and recovery codes. */
  factors(): Promise<FactorsView>;
  /** Offers a new authenticator app, confirmed by `confirmTotp`. */
  startTotp(): Promise<TotpSetup>;
  confirmTotp(code: string): Promise<FactorAdded>;
  removeTotp(): Promise<FactorsView>;
  startPasskey(): Promise<PasskeyStart>;
  addPasskey(ticket: string, name: string, credential: RegistrationJson): Promise<FactorAdded>;
  removePasskey(id: string): Promise<FactorsView>;
  /** Replaces the recovery codes; the new ones are shown once. */
  newRecoveryCodes(): Promise<string[]>;
}

const ME = "/api/v1/me";
const FACTORS = `${ME}/second-factors`;

/** The account API over a Vellum server. */
export function createHttpAccountApi(http: Http): AccountApi {
  return {
    tokens: () => http.get<TokenView[]>(`${ME}/tokens`),
    tokenPolicy: () => http.get<TokenPolicy>(`${ME}/token-policy`),
    createToken: (input) => http.send<CreatedToken>("POST", `${ME}/tokens`, input),
    async revokeToken(id) {
      await http.send("DELETE", `${ME}/tokens/${encodeURIComponent(id)}`);
    },
    factors: () => http.get<FactorsView>(FACTORS),
    startTotp: () => http.send<TotpSetup>("POST", `${FACTORS}/totp`),
    confirmTotp: (code) => http.send<FactorAdded>("POST", `${FACTORS}/totp/confirm`, { code }),
    removeTotp: () => http.send<FactorsView>("DELETE", `${FACTORS}/totp`),
    async startPasskey() {
      const ceremony = await http.send<PasskeyCeremony>("POST", `${FACTORS}/passkeys/options`);
      return { ticket: ceremony.ticket, options: ceremony.options as CreationOptionsJson };
    },
    addPasskey: (ticket, name, credential) =>
      http.send<FactorAdded>("POST", `${FACTORS}/passkeys`, { ticket, name, credential }),
    removePasskey: (id) =>
      http.send<FactorsView>("DELETE", `${FACTORS}/passkeys/${encodeURIComponent(id)}`),
    async newRecoveryCodes() {
      const fresh = await http.send<RecoveryCodes>("POST", `${FACTORS}/recovery-codes`);
      return fresh.recoveryCodes;
    },
  };
}

export {
  type CreatedToken,
  type FactorAdded,
  type FactorsView,
  type Scope,
  type TokenPolicy,
  type TokenView,
  type TotpSetup,
} from "../api/api.gen";
