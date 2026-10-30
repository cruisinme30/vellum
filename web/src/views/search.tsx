// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useMemo } from "react";
import { Link, useSearchParams } from "react-router";

import type { SearchFilters } from "../data/types";
import { useSearch, useWorkspace } from "../data/workspace-context";
import { ViewTitle } from "../help/help-link";
import { useT } from "../i18n/i18n";

import { PageList } from "./page-list";

function searchStatus(
  t: ReturnType<typeof useT>,
  query: string,
  hits: readonly unknown[] | undefined,
): string {
  if (query === "") return t("search-hint");
  if (hits === undefined) return t("search-working");
  return t("search-count", { count: hits.length });
}

/** The ages the "Modified" filter offers, in days. */
const AGES = [7, 30, 365] as const;

/** Reads the filters from the URL: `owner=me`, `modified=<days>`, `starred=1`, `space=<id>`. */
function filtersOf(parameters: URLSearchParams): SearchFilters {
  const days = Number(parameters.get("modified"));
  const space = parameters.get("space") ?? "";
  return {
    ...(parameters.get("owner") === "me" && { owner: "me" }),
    ...(AGES.includes(days as (typeof AGES)[number]) && { modifiedDays: days }),
    ...(parameters.get("starred") === "1" && { starred: true }),
    ...(space !== "" && { spaceId: space }),
  };
}

/** The filter bar under the search box (R-DR-9): owner, modified, location, starred. */
function SearchFilterBar({
  parameters,
  onChange,
}: {
  readonly parameters: URLSearchParams;
  readonly onChange: (name: string, value: string) => void;
}) {
  const t = useT();
  const workspace = useWorkspace();
  return (
    <fieldset className="vl-search-filters">
      <legend>{t("search-filters")}</legend>
      <label>
        {t("search-filter-owner")}
        <select
          value={parameters.get("owner") ?? ""}
          onChange={(event) => {
            onChange("owner", event.target.value);
          }}
        >
          <option value="">{t("search-filter-anyone")}</option>
          <option value="me">{t("search-filter-me")}</option>
        </select>
      </label>
      <label>
        {t("search-filter-modified")}
        <select
          value={parameters.get("modified") ?? ""}
          onChange={(event) => {
            onChange("modified", event.target.value);
          }}
        >
          <option value="">{t("search-filter-any-time")}</option>
          {AGES.map((days) => (
            <option key={days} value={String(days)}>
              {t("search-filter-days", { days })}
            </option>
          ))}
        </select>
      </label>
      <label>
        {t("search-filter-location")}
        <select
          value={parameters.get("space") ?? ""}
          onChange={(event) => {
            onChange("space", event.target.value);
          }}
        >
          <option value="">{t("search-filter-everywhere")}</option>
          {workspace.spaces.map((space) => (
            <option key={space.id} value={space.id}>
              {space.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        <input
          type="checkbox"
          checked={parameters.get("starred") === "1"}
          onChange={(event) => {
            onChange("starred", event.target.checked ? "1" : "");
          }}
        />
        {t("search-filter-starred")}
      </label>
    </fieldset>
  );
}

/** Search across page titles and bodies; the query and filters live in the URL. */
export function SearchView() {
  const t = useT();
  const id = useId();
  const [parameters, setParameters] = useSearchParams();
  const query = parameters.get("q") ?? "";
  const key = parameters.toString();
  // Keyed on the URL so the search hook sees one object per set of filters.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const filters = useMemo(() => filtersOf(parameters), [key]);
  const hits = useSearch("search", query, filters);
  const set = (name: string, value: string) => {
    const next = new URLSearchParams(parameters);
    if (value === "") next.delete(name);
    else next.set(name, value);
    setParameters(next, { replace: true });
  };
  return (
    <div className="vl-view">
      <ViewTitle title={t("nav-search")} help="drive.search" />
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
            set("q", event.target.value);
          }}
        />
      </div>
      <SearchFilterBar parameters={parameters} onChange={set} />
      <p className="vl-muted" role="status">
        {searchStatus(t, query, hits)}
      </p>
      {query !== "" && hits !== undefined && (
        <PageList items={hits} query={query} empty={t("search-empty", { query })} />
      )}
    </div>
  );
}

/** Inbox: pages that mention me. */
export function InboxView() {
  const t = useT();
  const workspace = useWorkspace();
  const hits = useSearch("mentions", workspace.me.handle);
  return (
    <div className="vl-view">
      <h1>{t("nav-inbox")}</h1>
      <p className="vl-muted">
        {t("inbox-intro", { handle: workspace.me.handle })}{" "}
        <Link to="/settings/notifications">{t("notify-open")}</Link>
      </p>
      {hits === undefined ? (
        <p className="vl-muted" role="status">
          {t("search-working")}
        </p>
      ) : (
        <PageList items={hits} empty={t("inbox-empty")} />
      )}
    </div>
  );
}
