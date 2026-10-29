// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { messageOf, useCollections } from "./context";
import {
  extrasOf,
  NEW_TYPES,
  RETYPE_TARGETS,
  settingsFor,
  TypeFields,
  useExtras,
  type Extras,
  type RetypeTarget,
} from "./property-fields";
import { FormError, SidePanel } from "./side-panel";
import type { CollectionInfo, NewPropertyType, PropertyInfo } from "./types";

/** Calls `action`, then reports its error or clears the last one. */
function report(action: Promise<unknown>, onError: (error?: string) => void, after?: () => void) {
  action.then(
    () => {
      onError();
      after?.();
    },
    (error: unknown) => {
      onError(messageOf(error));
    },
  );
}

/** What changed in one property's form, as an update. */
function changesOf(property: PropertyInfo, name: string, type: string, extras: Extras) {
  const retyped = type !== property.type;
  const settings = settingsFor(type, extras, property.options);
  const before = settingsFor(property.type, extrasOf(property), property.options);
  const settingsChanged = JSON.stringify(settings) !== JSON.stringify(before);
  return {
    ...(name.trim() === property.name ? {} : { name: name.trim() }),
    ...(retyped ? { type: type as RetypeTarget } : {}),
    ...(retyped || settingsChanged ? settings : {}),
  };
}

/** One property: rename it, change its type or settings, or remove it. */
function PropertyRow({
  info,
  property,
  labels,
  onError,
}: {
  readonly info: CollectionInfo;
  readonly property: PropertyInfo;
  readonly labels: Readonly<Record<string, string>>;
  readonly onError: (error?: string) => void;
}) {
  const t = useT();
  const store = useCollections();
  const [name, setName] = useState(property.name);
  const [type, setType] = useState<string>(property.type);
  const [extras, set] = useExtras(property);
  const [confirming, setConfirming] = useState(false);
  const fixed = property.type === "title" || property.type === "relation";
  const twoWay = property.relation?.back !== undefined;
  const save = () => {
    const changed = changesOf(property, name, type, extras);
    if (Object.keys(changed).length > 0) {
      report(
        store.updateProperty({ collection_id: info.id, key: property.key, ...changed }),
        onError,
      );
    }
  };
  return (
    <li className="vl-property-row">
      <form
        aria-label={property.name}
        onSubmit={(event) => {
          event.preventDefault();
          save();
        }}
      >
        <label className="vl-field">
          {t("collection-property-name", { type: labels[property.type] ?? property.type })}
          <input
            required
            maxLength={100}
            value={name}
            onChange={(event) => {
              setName(event.target.value);
            }}
          />
        </label>
        {!fixed && (
          <label className="vl-field">
            {t("collection-property-type")}
            <select
              value={type}
              onChange={(event) => {
                setType(event.target.value);
              }}
            >
              {RETYPE_TARGETS.map((kind) => (
                <option key={kind} value={kind}>
                  {labels[kind]}
                </option>
              ))}
            </select>
          </label>
        )}
        {type !== property.type && (
          <p className="vl-muted vl-hint">{t("collection-retype-hint")}</p>
        )}
        {type !== "relation" && <TypeFields info={info} type={type} extras={extras} set={set} />}
        <div className="vl-panel-actions">
          <button type="submit" className="vl-text-button">
            {t("collection-save")}
          </button>
          {property.type !== "title" && !confirming && (
            <button
              type="button"
              className="vl-icon-button"
              aria-label={t("collection-delete-property", { property: property.name })}
              onClick={() => {
                setConfirming(true);
              }}
            >
              <Icon name="delete" />
            </button>
          )}
          {confirming && (
            <button
              type="button"
              className="vl-text-button vl-danger"
              onClick={() => {
                report(
                  store.deleteProperty({ collection_id: info.id, key: property.key }),
                  onError,
                );
              }}
            >
              {twoWay
                ? t("collection-confirm-delete-relation", { property: property.name })
                : t("collection-confirm-delete-property", { property: property.name })}
            </button>
          )}
        </div>
      </form>
    </li>
  );
}

/** Adds a property at the end of the column order. */
function AddProperty({
  info,
  labels,
  onError,
}: {
  readonly info: CollectionInfo;
  readonly labels: Readonly<Record<string, string>>;
  readonly onError: (error?: string) => void;
}) {
  const t = useT();
  const store = useCollections();
  const [name, setName] = useState("");
  const [type, setType] = useState<NewPropertyType>("text");
  const [extras, set, setExtras] = useExtras();
  return (
    <form
      className="vl-row-fields vl-add-property"
      aria-label={t("collection-add-property")}
      onSubmit={(event) => {
        event.preventDefault();
        report(
          store.addProperty({
            collection_id: info.id,
            name: name.trim(),
            type,
            ...settingsFor(type, extras),
          }),
          onError,
          () => {
            setName("");
            setExtras(extrasOf());
          },
        );
      }}
    >
      <h3>{t("collection-add-property")}</h3>
      <label className="vl-field">
        {t("collection-new-property-name")}
        <input
          required
          maxLength={100}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
          }}
        />
      </label>
      <label className="vl-field">
        {t("collection-property-type")}
        <select
          value={type}
          onChange={(event) => {
            setType(event.target.value as NewPropertyType);
          }}
        >
          {NEW_TYPES.map((kind) => (
            <option key={kind} value={kind}>
              {labels[kind]}
            </option>
          ))}
        </select>
      </label>
      <TypeFields info={info} type={type} extras={extras} set={set} />
      <div className="vl-panel-actions">
        <button type="submit" className="vl-filled-button">
          {t("collection-add-property")}
        </button>
      </div>
    </form>
  );
}

/** Values that did not fit their property, kept in the files as written. */
function Problems({ info }: { readonly info: CollectionInfo }) {
  const t = useT();
  if (info.diagnostics.length === 0) return;
  return (
    <details className="vl-csv-report">
      <summary>{t("collection-problems", { count: info.diagnostics.length })}</summary>
      <ul>
        {info.diagnostics.slice(0, 50).map((d, index) => (
          <li key={`${String(index)}-${d.location}`}>{d.message}</li>
        ))}
      </ul>
    </details>
  );
}

/** A collection's properties: rename, retype, re-option, remove, and add new ones. */
export function PropertiesPanel({
  info,
  onClose,
}: {
  readonly info: CollectionInfo;
  readonly onClose: () => void;
}) {
  const t = useT();
  const [error, setError] = useState<string>();
  const labels: Readonly<Record<string, string>> = {
    title: t("collection-type-title"),
    text: t("collection-type-text"),
    number: t("collection-type-number"),
    select: t("collection-type-select"),
    multi: t("collection-type-multi"),
    person: t("collection-type-person"),
    date: t("collection-type-date"),
    checkbox: t("collection-type-checkbox"),
    url: t("collection-type-url"),
    relation: t("collection-type-relation"),
    rollup: t("collection-type-rollup"),
    formula: t("collection-type-formula"),
    created: t("collection-type-created"),
  };
  return (
    <SidePanel title={t("collection-properties")} onClose={onClose}>
      <FormError error={error} />
      <Problems info={info} />
      <ul className="vl-property-list">
        {info.properties.map((property) => (
          <PropertyRow
            // A new schema starts each row's draft over.
            key={JSON.stringify(property)}
            info={info}
            property={property}
            labels={labels}
            onError={setError}
          />
        ))}
      </ul>
      <AddProperty info={info} labels={labels} onError={setError} />
    </SidePanel>
  );
}
