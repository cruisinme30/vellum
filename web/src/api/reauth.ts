// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Re-authentication before admin actions (R-SE-7, ADR 0021 D241).
 *
 * Admin operations, and a few others such as creating an API token, need a
 * sign-in from the last few minutes. An older session gets `401
 * reauthentication required`. {@link withReauth} turns that answer into a
 * prompt: it asks the {@link ReauthGate}, which the app shows as a dialog,
 * and repeats the request once the person has proved who they are. Every
 * request waiting at the same time shares one prompt.
 */

import { ApiError, type Http } from "./http";

/** The server's message for a session that is too old for the action. */
export const REAUTHENTICATION_REQUIRED = "reauthentication required";

/** Whether `error` asks the person to sign in again before retrying. */
export function needsReauth(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    error.code === "unauthenticated" &&
    error.message === REAUTHENTICATION_REQUIRED
  );
}

/** Asks the person to confirm who they are; the dialog answers it. */
export interface ReauthGate {
  /** Resolves `true` once re-authenticated, `false` when the person cancels. */
  request(): Promise<boolean>;
  /** Whether a prompt is waiting for an answer. */
  pending(): boolean;
  /** Called whenever a prompt opens or closes. Returns the unsubscribe. */
  subscribe(listener: () => void): () => void;
  /** Ends the waiting prompt. */
  settle(confirmed: boolean): void;
}

export function createReauthGate(): ReauthGate {
  let waiting: { promise: Promise<boolean>; resolve: (ok: boolean) => void } | undefined;
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of listeners) listener();
  };
  return {
    request() {
      if (waiting === undefined) {
        let resolve!: (ok: boolean) => void;
        const promise = new Promise<boolean>((r) => {
          resolve = r;
        });
        waiting = { promise, resolve };
        notify();
      }
      return waiting.promise;
    },
    pending: () => waiting !== undefined,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    settle(confirmed) {
      const current = waiting;
      waiting = undefined;
      current?.resolve(confirmed);
      notify();
    },
  };
}

/**
 * The client with re-authentication: a request the server refuses for an
 * old sign-in waits for the gate, then runs once more. A cancelled prompt
 * fails the request with the server's error.
 */
export function withReauth(http: Http, gate: ReauthGate): Http {
  const retry = async <T>(run: () => Promise<T>): Promise<T> => {
    try {
      return await run();
    } catch (error) {
      if (!needsReauth(error) || !(await gate.request())) throw error;
      return run();
    }
  };
  return {
    get: (path, query) => retry(() => http.get(path, query)),
    send: (method, path, body) => retry(() => http.send(method, path, body)),
    upload: (path, query, body, type) => retry(() => http.upload(path, query, body, type)),
    forgetSession: () => {
      http.forgetSession();
    },
  };
}

/** Proves the password again; the session counts as a fresh sign-in after it. */
export async function reauthenticate(http: Http, password: string): Promise<void> {
  await http.send("POST", "/auth/reauthenticate", { password });
}

/** Where single sign-on re-authentication starts, returning to `next`. */
export function ssoReauthUrl(startUrl: string, next: string): string {
  const joiner = startUrl.includes("?") ? "&" : "?";
  return `${startUrl}${joiner}${new URLSearchParams({ reauth: "1", next }).toString()}`;
}
