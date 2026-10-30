// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useId, useState, type SyntheticEvent } from "react";
import { useSearchParams } from "react-router";

import { Ready } from "../data/server-apis";
import { useT } from "../i18n/i18n";

import type { DriveApi, Filters, NodeKindView, SpaceView } from "./api";
import { DriveFrame } from "./drive-page";
import { ItemTable } from "./item-table";
import { loadSpaces, useReloadable, withStars } from "./loading";

const KINDS: readonly NodeKindView[] = [
  "page",
  "folder",
  "book",
  "collection",
  "shortcut",
];

const isKind = (value: string): value is NodeKindView =>
  (KINDS as readonly string[]).includes(value);

/** The filters a Drive search URL holds. */
export function filtersOf(parameters: URLSearchParams): Filters {
  return {
    title: parameters.get("q") ?? undefined,
    kinds: parameters.getAll("kind").filter((kind) => isKind(kind)),
    after: parameters.get("after") ?? undefined,
    before: parameters.get("before") ?? undefined,
    location: parameters.get("in") ?? undefined,
    starred: parameters.get("starred") === "1",
  };
}

/**
 * The URL for some filters, so a search can be shared and the back button
 * works.
 */
export function parametersOf(filters: Filters): URLSearchParams {
  const parameters = new URLSearchParams();
  const set = (key: string, value: string | undefined) => {
    if (value !== undefined && value.trim() !== "")
      parameters.set(key, value.trim());
  };
  set("q", filters.title);
  for (const kind of filters.kinds ?? []) parameters.append("kind", kind);
  set("after", filters.after);
  set("before", filters.before);
  set("in", filters.location);
  if (filters.starred === true) parameters.set("starred", "1");
  return parameters;
}

const hasAny = (filters: Filters) => parametersOf(filters).size > 0;

function TextFilter({
  label,
  type,
  value,
  onChange,
}: {
  readonly label: string;
  readonly type: "search" | "date";
  readonly value: string | undefined;
  readonly onChange: (value: string) => void;
}) {
  const id = useId();
  return (
    <div className="vl-drive-field">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type={type}
        value={value ?? ""}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      />
    </div>
  );
}

/**
 * The filter form (R-DR-9): title, kind, modified range, location and starred.
 */
function FilterForm({
  spaces,
  initial,
  onSearch,
}: {
  readonly spaces: readonly SpaceView[];
  readonly initial: Filters;
  readonly onSearch: (filters: Filters) => void;
}) {
  const t = useT();
  const locationId = useId();
  const [draft, setDraft] = useState(initial);
  const change = (fields: Partial<Filters>) => {
    setDraft((current) => ({ ...current, ...fields }));
  };
  const kinds = new Set(draft.kinds);
  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    onSearch(draft);
  };
  return (
    <form className="vl-drive-filters" role="search" onSubmit={submit}>
      <TextFilter
        label={t("drive-filter-title")}
        type="search"
        value={draft.title}
        onChange={(title) => {
          change({ title });
        }}
      />
      <fieldset>
        <legend>{t("drive-filter-kind")}</legend>
        {KINDS.map((kind) => (
          <label key={kind}>
            <input
              type="checkbox"
              checked={kinds.has(kind)}
              onChange={(event) => {
                const next = KINDS.filter((k) =>
                  k === kind ? event.target.checked : kinds.has(k),
                );
                change({ kinds: next });
              }}
            />
            {t(`drive-kind-${kind}`)}
          </label>
        ))}
      </fieldset>
      <TextFilter
        label={t("drive-filter-after")}
        type="date"
        value={draft.after}
        onChange={(after) => {
          change({ after });
        }}
      />
      <TextFilter
        label={t("drive-filter-before")}
        type="date"
        value={draft.before}
        onChange={(before) => {
          change({ before });
        }}
      />
      <div className="vl-drive-field">
        <label htmlFor={locationId}>{t("drive-filter-location")}</label>
        <select
          id={locationId}
          value={draft.location ?? ""}
          onChange={(event) => {
            change({ location: event.target.value });
          }}
        >
          <option value="">{t("drive-filter-anywhere")}</option>
          {spaces.map((space) => (
            <option key={space.id} value={space.id}>
              {space.name}
            </option>
          ))}
        </select>
      </div>
      <label className="vl-drive-check">
        <input
          type="checkbox"
          checked={draft.starred === true}
          onChange={(event) => {
            change({ starred: event.target.checked });
          }}
        />
        {t("drive-filter-starred")}
      </label>
      <button type="submit" className="vl-filled-button">
        {t("drive-search")}
      </button>
    </form>
  );
}

function Results({
  api,
  spaces,
  filters,
}: {
  readonly api: DriveApi;
  readonly spaces: readonly SpaceView[];
  readonly filters: Filters;
}) {
  const t = useT();
  const key = parametersOf(filters).toString();
  const { loaded, reload } = useReloadable(
    () => withStars(api, api.find({ ...filters })),
    [api, key],
  );
  return (
    <Ready loaded={loaded}>
      {(listing) => (
        <ItemTable
          api={api}
          items={listing.items}
          starred={listing.starred}
          spaces={spaces}
          empty={t("drive-search-empty")}
          onChanged={reload}
        />
      )}
    </Ready>
  );
}

function DriveSearch({ api }: { readonly api: DriveApi }) {
  const t = useT();
  const [parameters, setParameters] = useSearchParams();
  const filters = filtersOf(parameters);
  const spaces = useReloadable(() => loadSpaces(api), [api]).loaded;
  return (
    <Ready loaded={spaces}>
      {(list) => (
        <>
          <FilterForm
            spaces={list}
            initial={filters}
            onSearch={(next) => {
              setParameters(parametersOf(next));
            }}
          />
          {hasAny(filters) ? (
            <Results api={api} spaces={list} filters={filters} />
          ) : (
            <p className="vl-muted">{t("drive-search-hint")}</p>
          )}
        </>
      )}
    </Ready>
  );
}

/** Drive search with filters; the filters live in the URL. */
export function DriveSearchPage() {
  return <DriveFrame>{(api) => <DriveSearch api={api} />}</DriveFrame>;
}
