// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, type ReactNode } from "react";

import { ViewTitle } from "../../help/help-link";
import type { HelpTopic } from "../../help/topics.gen";
import { useT } from "../../i18n/i18n";

import "./console.css";

import type { AdminConsoleApi } from "./api";

const AdminConsoleContext = createContext<AdminConsoleApi | undefined>(
  undefined,
);

/** Provides the admin console API; absent in demo mode. */
export function AdminConsoleProvider({
  api,
  children,
}: {
  readonly api: AdminConsoleApi | undefined;
  readonly children: ReactNode;
}) {
  return <AdminConsoleContext value={api}>{children}</AdminConsoleContext>;
}

/**
 * An admin screen's frame: its heading and, when the app runs without a
 * Vellum server, a note instead of the screen.
 */
export function AdminScreen({
  title,
  intro,
  help,
  children,
}: {
  readonly title: string;
  /** The help topic for the heading's `?` link. */
  readonly help?: HelpTopic | undefined;
  readonly intro: string;
  readonly children: (api: AdminConsoleApi) => ReactNode;
}) {
  const t = useT();
  const api = use(AdminConsoleContext);
  return (
    <section className="vl-view vl-admin">
      <ViewTitle title={title} help={help} />
      <p className="vl-muted">{intro}</p>
      {api === undefined ? <p>{t("admin-needs-server")}</p> : children(api)}
    </section>
  );
}
