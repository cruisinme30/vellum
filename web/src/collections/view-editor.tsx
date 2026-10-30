// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";

import { messageOf, useCollections } from "./context";
import { FormError, SidePanel } from "./side-panel";
import type { CollectionInfo, PropertyInfo, ViewInfo, ViewKind } from "./types";

const KINDS: readonly ViewKind[] = [
  "table",
  "board",
  "gallery",
  "calendar",
  "timeline",
  "list",
];

/** Shows `est > 1` as `Estimate > 1`: views store keys, people read names. */
export function withNames(
  line: string,
  properties: readonly PropertyInfo[],
): string {
  const trimmed = line.trim();
  const property = properties.find(
    (p) => trimmed === p.key || trimmed.startsWith(`${p.key} `),
  );
  return property
    ? `${property.name}${trimmed.slice(property.key.length)}`
    : trimmed;
}

const lines = (text: string) =>
  text
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l !== "");

interface Draft {
  name: string;
  kind: ViewKind;
  filters: string;
  sorts: string;
  group: string;
  date: string;
  end: string;
  hidden: readonly string[];
  limit: string;
}

const BLANK: ViewInfo = {
  name: "",
  kind: "table",
  filters: [],
  sorts: [],
  hidden: [],
};

function draftOf(
  view: ViewInfo = BLANK,
  properties: readonly PropertyInfo[],
): Draft {
  const named = (items: readonly string[]) =>
    items.map((item) => withNames(item, properties)).join("\n");
  return {
    name: view.name,
    kind: view.kind,
    filters: named(view.filters),
    sorts: named(view.sorts),
    group: view.group ?? "",
    date: view.date ?? "",
    end: view.end ?? "",
    hidden: view.hidden,
    limit: String(view.limit ?? ""),
  };
}

/** `{ [key]: value }`, or nothing for an empty value. */
const optional = (key: string, value: string) =>
  value === "" ? {} : { [key]: value };

function PropertySelect({
  label,
  value,
  options,
  onChange,
}: {
  readonly label: string;
  readonly value: string;
  readonly options: readonly PropertyInfo[];
  readonly onChange: (key: string) => void;
}) {
  const t = useT();
  return (
    <label className="vl-field">
      {label}
      <select
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        <option value="">{t("collection-none")}</option>
        {options.map((p) => (
          <option key={p.key} value={p.key}>
            {p.name}
          </option>
        ))}
      </select>
    </label>
  );
}

/**
 * Edits a saved view, or makes a new one when `view` is absent: layout,
 * filters, sorts, grouping, dates, hidden properties and row limit.
 */
export function ViewEditor({
  info,
  view,
  onSaved,
  onClose,
}: {
  readonly info: CollectionInfo;
  readonly view?: ViewInfo;
  /** Called with the saved view's name, or undefined after a delete. */
  readonly onSaved: (name?: string) => void;
  readonly onClose: () => void;
}) {
  const t = useT();
  const store = useCollections();
  const [draft, setDraft] = useState(() => draftOf(view, info.properties));
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => {
    setDraft((d) => ({ ...d, [key]: value }));
  };
  const dates = info.properties.filter(
    (p) => p.type === "date" || p.type === "created",
  );
  const groupable = info.properties.filter((p) =>
    [
      "select",
      "multi",
      "person",
      "checkbox",
      "relation",
      "text",
      "date",
    ].includes(p.type),
  );
  const hideable = info.properties.filter((p) => p.type !== "title");
  const timed = draft.kind === "calendar" || draft.kind === "timeline";
  const kindLabels: Readonly<Record<ViewKind, string>> = {
    table: t("collection-kind-table"),
    board: t("collection-kind-board"),
    gallery: t("collection-kind-gallery"),
    calendar: t("collection-kind-calendar"),
    timeline: t("collection-kind-timeline"),
    list: t("collection-kind-list"),
  };

  const run = async (action: () => Promise<unknown>, after: () => void) => {
    setBusy(true);
    try {
      await action();
      setError(undefined);
      after();
    } catch (error_) {
      setError(messageOf(error_));
    } finally {
      setBusy(false);
    }
  };
  const save = () => {
    const name = draft.name.trim();
    const limit = Number(draft.limit);
    void run(
      () =>
        store.saveView({
          collection_id: info.id,
          view: view?.name ?? name,
          ...(view !== undefined && name !== view.name
            ? { rename_to: name }
            : {}),
          kind: draft.kind,
          filter: lines(draft.filters),
          sort: lines(draft.sorts),
          hide: [...draft.hidden],
          ...optional("group", draft.group),
          ...(timed ? optional("date", draft.date) : {}),
          ...(draft.kind === "timeline" ? optional("end", draft.end) : {}),
          ...(draft.limit.trim() !== "" && Number.isInteger(limit) && limit > 0
            ? { limit }
            : {}),
        }),
      () => {
        onSaved(name);
      },
    );
  };
  const remove = () => {
    if (!view) return;
    void run(
      () => store.deleteView({ collection_id: info.id, view: view.name }),
      () => {
        onSaved();
      },
    );
  };

  return (
    <SidePanel
      title={
        view
          ? t("collection-edit-view", { view: view.name })
          : t("collection-new-view")
      }
      onClose={onClose}
    >
      <form
        className="vl-row-fields"
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <label className="vl-field">
          {t("collection-view-name")}
          <input
            required
            maxLength={100}
            value={draft.name}
            onChange={(event) => {
              set("name", event.target.value);
            }}
          />
        </label>
        <label className="vl-field">
          {t("collection-view-layout")}
          <select
            value={draft.kind}
            onChange={(event) => {
              set("kind", event.target.value as ViewKind);
            }}
          >
            {KINDS.map((kind) => (
              <option key={kind} value={kind}>
                {kindLabels[kind]}
              </option>
            ))}
          </select>
        </label>
        <label className="vl-field">
          {t("collection-view-filters")}
          <textarea
            rows={3}
            value={draft.filters}
            placeholder="Status is Done"
            aria-describedby={`${info.id}-filter-hint`}
            onChange={(event) => {
              set("filters", event.target.value);
            }}
          />
        </label>
        <p id={`${info.id}-filter-hint`} className="vl-muted vl-hint">
          {t("collection-view-filters-hint")}
        </p>
        <label className="vl-field">
          {t("collection-view-sorts")}
          <textarea
            rows={2}
            value={draft.sorts}
            placeholder="Due desc"
            onChange={(event) => {
              set("sorts", event.target.value);
            }}
          />
        </label>
        <PropertySelect
          label={t("collection-view-group")}
          value={draft.group}
          options={groupable}
          onChange={(key) => {
            set("group", key);
          }}
        />
        {timed && (
          <PropertySelect
            label={t("collection-view-date")}
            value={draft.date}
            options={dates}
            onChange={(key) => {
              set("date", key);
            }}
          />
        )}
        {draft.kind === "timeline" && (
          <PropertySelect
            label={t("collection-view-end")}
            value={draft.end}
            options={dates}
            onChange={(key) => {
              set("end", key);
            }}
          />
        )}
        <fieldset className="vl-field vl-check-list">
          <legend>{t("collection-view-hidden")}</legend>
          {hideable.map((p) => (
            <label key={p.key}>
              <input
                type="checkbox"
                checked={draft.hidden.includes(p.key)}
                onChange={(event) => {
                  set(
                    "hidden",
                    event.target.checked
                      ? [...draft.hidden, p.key]
                      : draft.hidden.filter((h) => h !== p.key),
                  );
                }}
              />
              {p.name}
            </label>
          ))}
        </fieldset>
        <label className="vl-field">
          {t("collection-view-limit")}
          <input
            type="number"
            min={1}
            inputMode="numeric"
            value={draft.limit}
            onChange={(event) => {
              set("limit", event.target.value);
            }}
          />
        </label>
        <FormError error={error} />
        <div className="vl-panel-actions">
          <button type="submit" className="vl-filled-button" disabled={busy}>
            {t("collection-save")}
          </button>
          {view && info.views.length > 1 && !confirming && (
            <button
              type="button"
              className="vl-text-button vl-danger"
              onClick={() => {
                setConfirming(true);
              }}
            >
              {t("collection-delete-view")}
            </button>
          )}
          {confirming && (
            <button
              type="button"
              className="vl-text-button vl-danger"
              disabled={busy}
              onClick={remove}
            >
              {t("collection-confirm-delete-view", { view: view?.name ?? "" })}
            </button>
          )}
        </div>
      </form>
    </SidePanel>
  );
}
