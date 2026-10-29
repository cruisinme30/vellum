// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { ApiError, type Http } from "./http";

/** `UserView`: a user as the API shows it. */
export interface User {
  readonly id: string;
  readonly email: string;
  readonly name: string;
  readonly handle: string;
  readonly admin: boolean;
}

/** `AuthStatus`: who is calling and what the workspace needs. */
export interface AuthStatus {
  readonly needsSetup: boolean;
  readonly credential: string;
  readonly scopes: readonly string[];
  readonly user?: User | null;
}

/** The first administrator's details (`SetupInput`). */
export interface SetupInput {
  readonly name: string;
  readonly email: string;
  /** At least 12 characters. */
  readonly password: string;
}

/** Sign-in, sign-out and first-run set-up, on top of the HTTP client. */
export interface AuthApi {
  /** `identity.status`; `undefined` when no Vellum server answers (demo mode). */
  status(): Promise<AuthStatus | undefined>;
  signIn(email: string, password: string): Promise<User | undefined>;
  signOut(): Promise<void>;
  /** Creates the first administrator, then signs them in. */
  setUp(input: SetupInput): Promise<User | undefined>;
}

const isStatus = (value: unknown): value is AuthStatus =>
  typeof value === "object" &&
  value !== null &&
  typeof (value as Record<string, unknown>)["needsSetup"] === "boolean";

export function createAuthApi(http: Http): AuthApi {
  const signIn = async (email: string, password: string): Promise<User | undefined> => {
    const result = await http.send<{ user?: User | null }>("POST", "/auth/sign-in", {
      email,
      password,
    });
    http.forgetSession();
    return result.user ?? undefined;
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
  };
}
