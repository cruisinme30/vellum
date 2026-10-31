// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  createContext,
  use,
  useCallback,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { Navigate, useLocation } from "react-router";

import type { AuthApi, AuthStatus } from "./auth";

/** The signed-in state when the app runs against a Vellum server. */
export interface Session {
  readonly auth: AuthApi;
  readonly status: AuthStatus;
  /** Reads who is signed in again, after signing in, out or setting up. */
  refresh(): Promise<void>;
}

const SessionContext = createContext<Session | undefined>(undefined);

/** Provides the session; without one the app runs on demo data. */
export function SessionProvider({
  auth,
  initial,
  children,
}: {
  readonly auth: AuthApi;
  readonly initial: AuthStatus;
  readonly children: ReactNode;
}) {
  const [status, setStatus] = useState(initial);
  const refresh = useCallback(async () => {
    const next = await auth.status();
    if (next !== undefined) setStatus(next);
  }, [auth]);
  const value = useMemo(
    () => ({ auth, status, refresh }),
    [auth, status, refresh],
  );
  return <SessionContext value={value}>{children}</SessionContext>;
}

/** The session, or `undefined` in demo mode (no server). */
export function useSession(): Session | undefined {
  return use(SessionContext);
}

/** Where to go after signing in: a path inside the app, never another site. */
export function safeNext(next: string | null): string {
  if (
    next === null ||
    !next.startsWith("/") ||
    next.startsWith("//") ||
    next.startsWith("/\\")
  ) {
    return "/";
  }
  return next;
}

/**
 * Lets its children through only when someone is signed in. A new
 * workspace goes to set-up first; anyone else to sign-in, coming back here
 * afterwards. In demo mode there is no server and nothing to check.
 */
export function RequireSession({ children }: { readonly children: ReactNode }) {
  const session = useSession();
  const location = useLocation();
  if (session === undefined) return <>{children}</>;
  if (session.status.needsSetup) return <Navigate to="/setup" replace />;
  if (session.status.user === undefined || session.status.user === null) {
    const next = `${location.pathname}${location.search}${location.hash}`;
    const query =
      next === "/" ? "" : `?${new URLSearchParams({ next }).toString()}`;
    return <Navigate to={`/sign-in${query}`} replace />;
  }
  return <>{children}</>;
}
