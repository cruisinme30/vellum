// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId } from "react";
import { Link, useSearchParams } from "react-router";

import { useServices, useWorkspace } from "../data/workspace-context";
import { useT } from "../i18n/i18n";

import { PageList } from "./page-list";

/** Search across page titles and bodies; the query lives in the URL. */
export function SearchView() {
  const t = useT();
  const id = useId();
  const { store } = useServices();
  useWorkspace();
  const [parameters, setParameters] = useSearchParams();
  const query = parameters.get("q") ?? "";
  const hits = store.search(query);
  return (
    <div className="vl-view">
      <h1>{t("nav-search")}</h1>
      <div role="search" className="vl-search">
        <label htmlFor={id}>{t("search-label")}</label>
        <input
          id={id}
          type="search"
          value={query}
          // The search page exists to type a query.
          // eslint-disable-next-line jsx-a11y/no-autofocus
          autoFocus
          onChange={(event) => {
            setParameters(event.target.value === "" ? {} : { q: event.target.value }, {
              replace: true,
            });
          }}
        />
      </div>
      <p className="vl-muted" role="status">
        {query === "" ? t("search-hint") : t("search-count", { count: hits.length })}
      </p>
      {query !== "" && <PageList items={hits} empty={t("search-empty", { query })} />}
    </div>
  );
}

/** Inbox: pages that mention me. */
export function InboxView() {
  const t = useT();
  const { store } = useServices();
  const workspace = useWorkspace();
  return (
    <div className="vl-view">
      <h1>{t("nav-inbox")}</h1>
      <p className="vl-muted">
        {t("inbox-intro", { handle: workspace.me.handle })}{" "}
        <Link to="/settings/notifications">{t("notify-open")}</Link>
      </p>
      <PageList items={store.mentions(workspace.me.handle)} empty={t("inbox-empty")} />
    </div>
  );
}
