// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useCallback, useMemo } from "react";

import { useWorkspace } from "../data/workspace-context";

import { messageOf, useCollections, useLoad } from "./context";
import type { Column, PropertyInfo, PropertyType, Value } from "./types";
import { EMPTY, list, text, texts } from "./value";

/** One entry of a select, person or relation picker. */
export interface Choice {
  /** The stored text: an option name, a handle or a row id. */
  readonly id: string;
  readonly label: string;
  readonly color?: string;
}

const CHOICE_TYPES = new Set<PropertyType>([
  "select",
  "multi",
  "person",
  "relation",
]);

/** Whether a property is edited by picking from a list. */
export const isChoice = (type: PropertyType) => CHOICE_TYPES.has(type);

/** Whether a property holds several picks. */
export const isMultiple = (type: PropertyType) =>
  type === "multi" || type === "relation";

/** The value a set of picks stores. */
export function choiceValue(type: PropertyType, ids: readonly string[]): Value {
  if (ids.length === 0) {
    return EMPTY;
  }
  return isMultiple(type)
    ? list(ids.map((id) => text(id)))
    : text(ids[0] ?? "");
}

/** The value that puts a row in a group: lists hold the one pick. */
export function groupValue(type: PropertyType, value: Value | null): Value {
  if (value === null) {
    return EMPTY;
  }
  return isMultiple(type) ? list([value]) : value;
}

/**
 * A property as an editor needs it, from the full schema or a result column.
 */
export function propertyOf(
  properties: readonly PropertyInfo[],
  column: Column,
): PropertyInfo {
  return (
    properties.find((p) => p.key === column.key) ?? {
      key: column.key,
      name: column.name,
      type: column.type,
      computed: !column.editable,
      ...(column.options ? { options: column.options } : {}),
    }
  );
}

/** Display names by person handle. */
export function usePeople(): ReadonlyMap<string, string> {
  const { people } = useWorkspace();
  return useMemo(
    () => new Map(people.map((p) => [p.handle, p.name])),
    [people],
  );
}

/** What a picker offers for a property: options, people or related rows. */
export function useChoices(property: PropertyInfo): readonly Choice[] {
  const { people } = useWorkspace();
  const target =
    property.type === "relation" ? (property.relation?.to ?? "") : "";
  const related = useLoad(`choices:${target}`, async (store) => {
    if (target === "") {
      return [];
    }
    const result = await store.query({ collection_id: target });
    return result.groups.flatMap((g) =>
      g.rows.map((r) => ({ id: r.id, label: r.title })),
    );
  });
  if (property.type === "person") {
    return people.map((p) => ({ id: p.handle, label: p.name }));
  }
  if (property.type === "relation") {
    return related.data ?? [];
  }
  return (property.options ?? []).map((o) => ({
    id: o.name,
    label: o.name,
    ...(o.color === undefined ? {} : { color: o.color }),
  }));
}

/** The labels of the picks a value holds, by choice id. */
export function choiceLabels(
  choices: readonly Choice[],
  value: Value | undefined,
): string[] {
  return texts(value).map(
    (id) => choices.find((c) => c.id === id)?.label ?? id,
  );
}

/**
 * Writes values to a row. Resolves to the error message when the write was
 * refused, so editors can show it next to the field.
 */
export function useWriter(collectionId: string) {
  const store = useCollections();
  return useCallback(
    async (
      rowId: string,
      values: Readonly<Record<string, Value>>,
      body?: string,
    ) => {
      try {
        await store.updateRow({
          collection_id: collectionId,
          row_id: rowId,
          values,
          ...(body === undefined ? {} : { body }),
        });
        return;
      } catch (error) {
        return messageOf(error);
      }
    },
    [store, collectionId],
  );
}
