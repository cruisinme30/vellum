// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, type ReactNode } from "react";

import type { AccountApi } from "../account/api";
import type { AssistApi, McpSetupApi } from "../assist/api";
import type { AttachmentsApi } from "../attachments/api";
import type { BooksApi } from "../books/api";
import type { DriveApi } from "../drive/api";
import { ViewTitle } from "../help/help-link";
import type { HelpTopic } from "../help/topics.gen";
import { useT } from "../i18n/i18n";
import type { DirectoryApi } from "../people/directory";
import type { PrivateSpacesApi } from "../platform/private/api";
import type { RenderApi } from "../render/api";
import type { SharingApi } from "../sharing/api";
import { CopyDiagnostic } from "../shell/copy-diagnostic";
import type { WorkApi } from "../work/api";
import type { PlanningApi } from "../work/planning/api";

import type { Loaded } from "./use-loaded";

/** Server APIs with no demo version; each is absent in demo mode. */
export interface ServerApis {
  readonly work?: WorkApi | undefined;
  readonly books?: BooksApi | undefined;
  readonly drive?: DriveApi | undefined;
  readonly attachments?: AttachmentsApi | undefined;
  readonly privateSpaces?: PrivateSpacesApi | undefined;
  /** People (and groups) to pick for access and sharing. */
  readonly directory?: DirectoryApi | undefined;
  /** The server's renderer: rendered pages and blocks. */
  readonly render?: RenderApi | undefined;
  /** Who has access to an item, and share links. */
  readonly sharing?: SharingApi | undefined;
  /** The signed-in person's API tokens and sign-in factors. */
  readonly account?: AccountApi | undefined;
  /** The person's own model: summarise, ask, draft (ADR 0056). */
  readonly assist?: AssistApi | undefined;
  /** The admin page's MCP client setup. */
  readonly mcpSetup?: McpSetupApi | undefined;
  readonly planning?: PlanningApi | undefined;
}

const ServerApisContext = createContext<ServerApis>({});

/** Provides the server-only APIs. */
export function ServerApisProvider({
  apis,
  children,
}: {
  readonly apis: ServerApis;
  readonly children?: ReactNode;
}) {
  return <ServerApisContext value={apis}>{children}</ServerApisContext>;
}

/** The server-only APIs; each is `undefined` in demo mode. */
export function useServerApis(): ServerApis {
  return use(ServerApisContext);
}

/** One server API, or `undefined` in demo mode. */
export function useServerApi<K extends keyof ServerApis>(name: K): ServerApis[K] {
  return use(ServerApisContext)[name];
}

/**
 * A screen's frame: its heading, and either the screen or, when the app runs
 * without a Vellum server, a note saying the screen needs one.
 */
export function ServerScreen<K extends keyof ServerApis>({
  api,
  title,
  help,
  className,
  children,
}: {
  readonly api: K;
  /** The heading; leave it out when the screen names itself once loaded. */
  readonly title?: string;
  /** The help topic for the heading's `?` link. */
  readonly help?: HelpTopic | undefined;
  readonly className?: string;
  readonly children: (api: NonNullable<ServerApis[K]>) => ReactNode;
}) {
  const t = useT();
  const found = use(ServerApisContext)[api];
  return (
    <section className={className === undefined ? "vl-view" : `vl-view ${className}`}>
      {title !== undefined && <ViewTitle title={title} help={help} />}
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
        <CopyDiagnostic message={loaded.message} />
      </p>
    );
  }
  return <>{children(loaded.value)}</>;
}
