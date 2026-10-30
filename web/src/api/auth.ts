// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { AssertionJson, RegistrationJson } from "../account/webauthn";

import type { TotpSetup } from "./api.gen";
import { ApiError, type Http } from "./http";

/** `UserView`: a user as the API shows it. */
export interface User {
  readonly id: string;
  readonly email: string;
  readonly name: string;
  readonly handle: string;
  readonly admin: boolean;
  /** Whether the account has a password; without one it signs in with SSO only. */
  readonly hasPassword?: boolean;
}

/** `SsoButton`: how single sign-on is offered, when a provider is set up. */
interface SsoButton {
  readonly name: string;
  /** Where the button goes; append `?next=<path>` to return there. */
  readonly startUrl: string;
}

/** `AuthStatus`: who is calling and what the workspace needs. */
export interface AuthStatus {
  readonly needsSetup: boolean;
  readonly credential: string;
  readonly scopes: readonly string[];
  readonly user?: User | null;
  readonly sso?: SsoButton | null;
}

/** The first administrator's details (`SetupInput`). */
interface SetupInput {
  readonly name: string;
  readonly email: string;
  /** At least 12 characters. */
  readonly password: string;
}

/**
 * The password was right, and a second step remains (issue #523, D290):
 * `verify` with a code or passkey, or `enrol` a first factor because the
 * workspace requires one. The ticket works for five minutes.
 */
export class SecondStepRequired extends Error {
  constructor(
    readonly step: "verify" | "enrol",
    readonly ticket: string,
    readonly methods: readonly string[],
  ) {
    super(step === "verify" ? "second factor required" : "second factor setup required");
    this.name = "SecondStepRequired";
  }
}

/** A code, or a passkey's answer, for the second step. */
interface SecondStepProof {
  readonly code?: string;
  readonly passkey?: AssertionJson;
}

/** A first factor added at sign-in. */
type Enrolment =
  | { readonly code: string }
  | { readonly passkey: { readonly name: string; readonly credential: RegistrationJson } };

/** The second step of a sign-in, against a ticket. */
export interface SignInSteps {
  /** Redeems the ticket with a code, recovery code or passkey; signs in. */
  verify(ticket: string, proof: SecondStepProof): Promise<User | undefined>;
  /** The passkey options for the ticket: sign with one, or create one to enrol. */
  passkeyOptions(ticket: string): Promise<unknown>;
  /** An authenticator app to set up with an enrolment ticket. */
  authenticator(ticket: string): Promise<TotpSetup>;
  /** Adds the first factor and signs in; the recovery codes are shown once. */
  enrol(
    ticket: string,
    enrolment: Enrolment,
  ): Promise<{ readonly user: User | undefined; readonly recoveryCodes: readonly string[] }>;
}

/** Sign-in, sign-out and first-run set-up, on top of the HTTP client. */
export interface AuthApi {
  /** `identity.status`; `undefined` when no Vellum server answers (demo mode). */
  status(): Promise<AuthStatus | undefined>;
  /** Signs in, or throws {@link SecondStepRequired} when a second step remains. */
  signIn(email: string, password: string): Promise<User | undefined>;
  signOut(): Promise<void>;
  /** Creates the first administrator, then signs them in. */
  setUp(input: SetupInput): Promise<User | undefined>;
  /** The second step; absent in tests that do not need it. */
  readonly steps?: SignInSteps;
}

interface SignInAnswer {
  readonly user?: User | null;
  readonly secondFactor?: { readonly ticket: string; readonly methods: readonly string[] };
  readonly enrol?: { readonly ticket: string; readonly methods: readonly string[] };
  readonly recoveryCodes?: readonly string[];
}

const isStatus = (value: unknown): value is AuthStatus =>
  typeof value === "object" &&
  value !== null &&
  typeof (value as Record<string, unknown>)["needsSetup"] === "boolean";

export function createAuthApi(http: Http): AuthApi {
  const signIn = async (email: string, password: string): Promise<User | undefined> => {
    const result = await http.send<SignInAnswer>("POST", "/auth/sign-in", {
      email,
      password,
    });
    if (result.secondFactor !== undefined) {
      throw new SecondStepRequired(
        "verify",
        result.secondFactor.ticket,
        result.secondFactor.methods,
      );
    }
    if (result.enrol !== undefined) {
      throw new SecondStepRequired("enrol", result.enrol.ticket, result.enrol.methods);
    }
    http.forgetSession();
    return result.user ?? undefined;
  };
  const steps: SignInSteps = {
    async verify(ticket, proof) {
      const result = await http.send<SignInAnswer>("POST", "/auth/sign-in/verify", {
        ticket,
        ...proof,
      });
      http.forgetSession();
      return result.user ?? undefined;
    },
    async passkeyOptions(ticket) {
      const result = await http.send<{ options: unknown }>(
        "POST",
        "/auth/sign-in/passkey-options",
        {
          ticket,
        },
      );
      return result.options;
    },
    authenticator: (ticket) =>
      http.send<TotpSetup>("POST", "/auth/sign-in/authenticator", { ticket }),
    async enrol(ticket, enrolment) {
      const result = await http.send<SignInAnswer>("POST", "/auth/sign-in/enrol", {
        ticket,
        ...enrolment,
      });
      http.forgetSession();
      return { user: result.user ?? undefined, recoveryCodes: result.recoveryCodes ?? [] };
    },
  };
  return {
    async status() {
      try {
        const status: unknown = await http.get("/api/v1/auth/status");
        return isStatus(status) ? status : undefined;
      } catch (error) {
        // Only "nobody is there" means demo mode; a real server's error is real.
        if (error instanceof ApiError && (error.code === "network" || error.status === 404)) {
          return;
        }
        throw error;
      }
    },
    signIn,
    async signOut() {
      await http.send("POST", "/auth/sign-out");
      http.forgetSession();
    },
    async setUp(input) {
      await http.send("POST", "/api/v1/setup", input);
      return signIn(input.email, input.password);
    },
    steps,
  };
}
