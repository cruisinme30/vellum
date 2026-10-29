// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, type ReactNode } from "react";

import type { BooksApi } from "../books/api";
import { useT } from "../i18n/i18n";
import type { PrivateSpacesApi } from "../platform/private/api";
import type { WorkApi } from "../work/api";
import type { PlanningApi } from "../work/planning/api";

import type { Loaded } from "./use-loaded";

/** Server APIs with no demo version; each is absent in demo mode. */
export interface ServerApis {
  readonly work?: WorkApi | undefined;
  readonly books?: BooksApi | undefined;
  readonly privateSpaces?: PrivateSpacesApi | undefined;
  readonly planning?: PlanningApi | undefined;
}

const ServerApisContext = createContext<ServerApis>({});

/** Provides the server-only APIs. */
export function ServerApisProvider({
  apis,
  children,
}: {
  readonly apis: ServerApis;
  readonly children: ReactNode;
}) {
  return <ServerApisContext value={apis}>{children}</ServerApisContext>;
}

/** One server API, or `undefined` in demo mode; for parts of a screen. */
export function useServerApi<K extends keyof ServerApis>(api: K): ServerApis[K] {
  return use(ServerApisContext)[api];
}

/**
 * A screen's frame: its heading, and either the screen or, when the app runs
 * without a Vellum server, a note saying the screen needs one.
 */
export function ServerScreen<K extends keyof ServerApis>({
  api,
  title,
  className,
  children,
}: {
  readonly api: K;
  /** The heading; leave it out when the screen names itself once loaded. */
  readonly title?: string;
  readonly className?: string;
  readonly children: (api: NonNullable<ServerApis[K]>) => ReactNode;
}) {
  const t = useT();
  const found = use(ServerApisContext)[api];
  return (
    <section className={className === undefined ? "vl-view" : `vl-view ${className}`}>
      {title !== undefined && <h1>{title}</h1>}
      {found === undefined ? <p>{t("needs-server")}</p> : children(found)}
    </section>
  );
}

/** A load's state: a loading line, the failure, or `children` with the value. */
export function Ready<T>({
  loaded,
  children,
}: {
  readonly loaded: Loaded<T>;
  readonly children: (value: T) => ReactNode;
}) {
  const t = useT();
  if (loaded.state === "loading") {
    return (
      <p className="vl-muted" role="status">
        {t("loading")}
      </p>
    );
  }
  if (loaded.state === "failed") {
    return (
      <p className="vl-field-error" role="alert">
        {loaded.message}
      </p>
    );
  }
  return <>{children(loaded.value)}</>;
}
