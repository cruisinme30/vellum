// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createContext, use, type ReactNode } from "react";

import { useT } from "../../i18n/i18n";

import "./console.css";

import type { AdminConsoleApi } from "./api";

const AdminConsoleContext = createContext<AdminConsoleApi | undefined>(undefined);

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
  children,
}: {
  readonly title: string;
  readonly intro: string;
  readonly children: (api: AdminConsoleApi) => ReactNode;
}) {
  const t = useT();
  const api = use(AdminConsoleContext);
  return (
    <section className="vl-view vl-admin">
      <h1>{title}</h1>
      <p className="vl-muted">{intro}</p>
      {api === undefined ? <p>{t("admin-needs-server")}</p> : children(api)}
    </section>
  );
}
