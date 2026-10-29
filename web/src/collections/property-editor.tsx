// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { messageOf, useCollections } from "./context";
import { FormError, SidePanel } from "./side-panel";
import type { CollectionInfo, NewPropertyType, PropertyInfo, SelectOption } from "./types";

const NEW_TYPES: readonly NewPropertyType[] = [
  "text",
  "number",
  "select",
  "multi",
  "person",
  "date",
  "checkbox",
  "url",
  "formula",
  "created",
];

/** `Draft, Live` → options, keeping the colours of options that stay. */
function optionsOf(text: string, old: readonly SelectOption[] = []): SelectOption[] {
  const names = [...new Set(text.split(",").map((s) => s.trim()))].filter((s) => s !== "");
  return names.map((name) => old.find((o) => o.name === name) ?? { name });
}

const hasOptions = (type: string) => type === "select" || type === "multi";

/** One property: rename it, change its options or formula, or remove it. */
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
  const [options, setOptions] = useState((property.options ?? []).map((o) => o.name).join(", "));
  const [formula, setFormula] = useState(property.formula ?? "");
  const [confirming, setConfirming] = useState(false);
  const removable = property.type !== "title" && property.relation?.back === undefined;
  const run = (action: () => Promise<unknown>) => {
    action().then(
      () => {
        onError();
      },
      (error: unknown) => {
        onError(messageOf(error));
      },
    );
  };
  const save = () => {
    const nextOptions = optionsOf(options, property.options);
    const changed = {
      ...(name.trim() === property.name ? {} : { name: name.trim() }),
      ...(hasOptions(property.type) &&
      nextOptions.map((o) => o.name).join("\n") !==
        (property.options ?? []).map((o) => o.name).join("\n")
        ? { options: nextOptions }
        : {}),
      ...(property.type === "formula" && formula !== (property.formula ?? "") ? { formula } : {}),
    };
    if (Object.keys(changed).length > 0) {
      run(() => store.updateProperty({ collection_id: info.id, key: property.key, ...changed }));
    }
  };
  return (
    <li className="vl-property-row">
      <form
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
        {hasOptions(property.type) && (
          <label className="vl-field">
            {t("collection-property-options")}
            <input
              value={options}
              onChange={(event) => {
                setOptions(event.target.value);
              }}
            />
          </label>
        )}
        {property.type === "formula" && (
          <label className="vl-field">
            {t("collection-property-formula")}
            <input
              value={formula}
              spellCheck={false}
              onChange={(event) => {
                setFormula(event.target.value);
              }}
            />
          </label>
        )}
        <div className="vl-panel-actions">
          <button type="submit" className="vl-text-button">
            {t("collection-save")}
          </button>
          {removable && !confirming && (
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
                run(() => store.deleteProperty({ collection_id: info.id, key: property.key }));
              }}
            >
              {t("collection-confirm-delete-property", { property: property.name })}
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
  const [options, setOptions] = useState("");
  const [formula, setFormula] = useState("");
  return (
    <form
      className="vl-row-fields vl-add-property"
      aria-label={t("collection-add-property")}
      onSubmit={(event) => {
        event.preventDefault();
        store
          .addProperty({
            collection_id: info.id,
            name: name.trim(),
            type,
            ...(hasOptions(type) ? { options: optionsOf(options) } : {}),
            ...(type === "formula" ? { formula } : {}),
          })
          .then(
            () => {
              onError();
              setName("");
              setOptions("");
              setFormula("");
            },
            (error: unknown) => {
              onError(messageOf(error));
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
      {hasOptions(type) && (
        <label className="vl-field">
          {t("collection-property-options")}
          <input
            value={options}
            placeholder="Draft, Live"
            onChange={(event) => {
              setOptions(event.target.value);
            }}
          />
        </label>
      )}
      {type === "formula" && (
        <label className="vl-field">
          {t("collection-property-formula")}
          <input
            required
            value={formula}
            spellCheck={false}
            placeholder="round(Estimate * 0.5, 1)"
            onChange={(event) => {
              setFormula(event.target.value);
            }}
          />
        </label>
      )}
      <div className="vl-panel-actions">
        <button type="submit" className="vl-filled-button">
          {t("collection-add-property")}
        </button>
      </div>
    </form>
  );
}

/** A collection's properties: rename, re-option, remove, and add new ones. */
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
      <ul className="vl-property-list">
        {info.properties.map((property) => (
          <PropertyRow
            // A new schema starts each row's draft over.
            key={`${property.key}:${property.name}:${(property.options ?? []).map((o) => o.name).join(",")}:${property.formula ?? ""}`}
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
