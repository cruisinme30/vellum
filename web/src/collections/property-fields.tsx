// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";

import { useLoad } from "./context";
import type {
  Aggregation,
  CollectionInfo,
  NewPropertyType,
  PropertyInfo,
  PropertyType,
  SelectOption,
} from "./types";

/** Types a property can be changed to: not a title or relation. */
export type RetypeTarget = Exclude<PropertyType, "title" | "relation">;

export const RETYPE_TARGETS: readonly RetypeTarget[] = [
  "text",
  "number",
  "select",
  "multi",
  "person",
  "date",
  "checkbox",
  "url",
  "rollup",
  "formula",
  "created",
];

export const NEW_TYPES: readonly NewPropertyType[] = [...RETYPE_TARGETS, "relation"];

const AGGREGATIONS: readonly Aggregation[] = [
  "count",
  "sum",
  "avg",
  "min",
  "max",
  "checked",
  "pct",
  "unique",
  "show",
];

/** `Draft, Live` → options, keeping the colours of options that stay. */
function optionsOf(text: string, old: readonly SelectOption[] = []): SelectOption[] {
  const names = [...new Set(text.split(",").map((s) => s.trim()))].filter((s) => s !== "");
  return names.map((name) => old.find((o) => o.name === name) ?? { name });
}

/** The settings a type needs, while they are typed in. */
export interface Extras {
  readonly options: string;
  readonly formula: string;
  /** A rollup's relation key. */
  readonly via: string;
  /** A rollup's property key in the related collection. */
  readonly prop: string;
  readonly agg: Aggregation;
  /** A relation's target collection. */
  readonly to: string;
  readonly backName: string;
}

const BLANK: PropertyInfo = { key: "", name: "", type: "text", computed: false };

export function extrasOf(property: PropertyInfo = BLANK): Extras {
  const rollup = property.rollup ?? { via: "", prop: "", agg: "count" };
  return {
    options: (property.options ?? []).map((o) => o.name).join(", "),
    formula: property.formula ?? "",
    via: rollup.via,
    prop: rollup.prop,
    agg: rollup.agg,
    to: "",
    backName: "",
  };
}

export type SetExtra = <K extends keyof Extras>(key: K, value: Extras[K]) => void;

/** What the server needs to know about a type's settings. */
export function settingsFor(type: string, extras: Extras, old?: readonly SelectOption[]) {
  switch (type) {
    case "select":
    case "multi": {
      return { options: optionsOf(extras.options, old) };
    }
    case "formula": {
      return { formula: extras.formula };
    }
    case "rollup": {
      return { via: extras.via, prop: extras.prop, agg: extras.agg };
    }
    case "relation": {
      return relationSettings(extras);
    }
    default: {
      return {};
    }
  }
}

function relationSettings(extras: Extras) {
  const back = extras.backName.trim();
  return { to: extras.to, ...(back === "" ? {} : { back_name: back }) };
}

/** What the rollup fields show before a relation is picked. */
const NO_TARGET: Pick<CollectionInfo, "properties"> = { properties: [] };

/** Pick a relation, a property of the collection it links to, and how to combine it. */
function RollupFields({
  info,
  extras,
  set,
}: {
  readonly info: CollectionInfo;
  readonly extras: Extras;
  readonly set: SetExtra;
}) {
  const t = useT();
  const relations = info.properties.filter((p) => p.type === "relation");
  const to = relations.find((p) => p.key === extras.via)?.relation?.to ?? "";
  const target = useLoad(`info:${to}`, (store) =>
    to === "" ? Promise.resolve(NO_TARGET) : store.info(to),
  );
  const labels: Readonly<Record<Aggregation, string>> = {
    count: t("collection-agg-count"),
    sum: t("collection-agg-sum"),
    avg: t("collection-agg-avg"),
    min: t("collection-agg-min"),
    max: t("collection-agg-max"),
    checked: t("collection-agg-checked"),
    pct: t("collection-agg-pct"),
    unique: t("collection-agg-unique"),
    show: t("collection-agg-show"),
  };
  if (relations.length === 0) {
    return <p className="vl-muted">{t("collection-rollup-needs-relation")}</p>;
  }
  return (
    <>
      <label className="vl-field">
        {t("collection-rollup-via")}
        <select
          required
          value={extras.via}
          onChange={(event) => {
            set("via", event.target.value);
            set("prop", "");
          }}
        >
          <option value="">{t("collection-none")}</option>
          {relations.map((p) => (
            <option key={p.key} value={p.key}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label className="vl-field">
        {t("collection-rollup-prop")}
        <select
          required
          value={extras.prop}
          onChange={(event) => {
            set("prop", event.target.value);
          }}
        >
          <option value="">{t("collection-none")}</option>
          {(target.data?.properties ?? []).map((p) => (
            <option key={p.key} value={p.key}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label className="vl-field">
        {t("collection-rollup-agg")}
        <select
          value={extras.agg}
          onChange={(event) => {
            set("agg", event.target.value as Aggregation);
          }}
        >
          {AGGREGATIONS.map((agg) => (
            <option key={agg} value={agg}>
              {labels[agg]}
            </option>
          ))}
        </select>
      </label>
    </>
  );
}

/** Pick the collection a relation links to, and name its pair there. */
function RelationFields({
  info,
  extras,
  set,
}: {
  readonly info: CollectionInfo;
  readonly extras: Extras;
  readonly set: SetExtra;
}) {
  const t = useT();
  const list = useLoad("collections", (store) => store.list());
  return (
    <>
      <label className="vl-field">
        {t("collection-relation-to")}
        <select
          required
          value={extras.to}
          onChange={(event) => {
            set("to", event.target.value);
          }}
        >
          <option value="">{t("collection-none")}</option>
          {(list.data?.collections ?? []).map((c) => (
            <option key={c.id} value={c.id}>
              {c.title}
            </option>
          ))}
        </select>
      </label>
      <label className="vl-field">
        {t("collection-relation-back")}
        <input
          maxLength={100}
          value={extras.backName}
          placeholder={info.title}
          onChange={(event) => {
            set("backName", event.target.value);
          }}
        />
      </label>
      <p className="vl-muted vl-hint">{t("collection-relation-hint")}</p>
    </>
  );
}

/** The settings fields a type needs. */
export function TypeFields({
  info,
  type,
  extras,
  set,
}: {
  readonly info: CollectionInfo;
  readonly type: string;
  readonly extras: Extras;
  readonly set: SetExtra;
}) {
  const t = useT();
  switch (type) {
    case "select":
    case "multi": {
      return (
        <label className="vl-field">
          {t("collection-property-options")}
          <input
            value={extras.options}
            placeholder="Draft, Live"
            onChange={(event) => {
              set("options", event.target.value);
            }}
          />
        </label>
      );
    }
    case "formula": {
      return (
        <label className="vl-field">
          {t("collection-property-formula")}
          <input
            required
            value={extras.formula}
            spellCheck={false}
            placeholder="round(Estimate * 0.5, 1)"
            onChange={(event) => {
              set("formula", event.target.value);
            }}
          />
        </label>
      );
    }
    case "rollup": {
      return <RollupFields info={info} extras={extras} set={set} />;
    }
    case "relation": {
      return <RelationFields info={info} extras={extras} set={set} />;
    }
    default: {
      return;
    }
  }
}

export function useExtras(property?: PropertyInfo) {
  const [extras, setExtras] = useState<Extras>(() => extrasOf(property));
  const set: SetExtra = (key, value) => {
    setExtras((current) => ({ ...current, [key]: value }));
  };
  return [extras, set, setExtras] as const;
}
