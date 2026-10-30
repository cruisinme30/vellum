// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Builds an AND/OR filter over every issue field without a query language
 * (D15): a group of conditions, each group joined by "all" or "any", and
 * groups nested one level for "(A or B) and C".
 */

import { useId, useState, type ComponentProps } from "react";

import { useT } from "../i18n/i18n";

import {
  EVERYTHING,
  FIELDS,
  newCondition,
  newGroup,
  opsFor,
  splitValues,
  suggestionsFor,
  takesValues,
  type Condition,
  type Field,
  type FilterNode,
  type Group,
  type Op,
} from "./filter";

type Path = readonly number[];

function replaceAt(
  group: Group,
  path: Path,
  next: FilterNode | undefined,
): Group {
  const [head, ...rest] = path;
  if (head === undefined) return group;
  const children = [...group.children];
  const child = children[head];
  if (child === undefined) return group;
  if (rest.length === 0) {
    if (next === undefined) children.splice(head, 1);
    else children[head] = next;
  } else if (child.kind === "group") {
    children[head] = replaceAt(child, rest, next);
  }
  return { ...group, children };
}

function ConditionRow({
  condition,
  statuses,
  onChange,
  onRemove,
}: {
  readonly condition: Condition;
  readonly statuses: readonly string[];
  readonly onChange: (next: Condition) => void;
  readonly onRemove: () => void;
}) {
  const t = useT();
  const id = useId();
  const [text, setText] = useState(condition.values.join(", "));
  const ops = opsFor(condition.field);
  const suggestions = suggestionsFor(condition.field, statuses);
  return (
    <li className="vl-filter-condition">
      <select
        aria-label={t("work-filter-field")}
        value={condition.field}
        onChange={(event) => {
          const field = event.target.value as Field;
          const fresh = newCondition(field);
          setText(fresh.values.join(", "));
          onChange(fresh);
        }}
      >
        {FIELDS.map((field) => (
          <option key={field} value={field}>
            {t(`work-field-${field}`)}
          </option>
        ))}
      </select>
      <select
        aria-label={t("work-filter-op")}
        value={condition.op}
        onChange={(event) => {
          onChange({ ...condition, op: event.target.value as Op });
        }}
      >
        {ops.map((op) => (
          <option key={op} value={op}>
            {t(`work-op-${op}`)}
          </option>
        ))}
      </select>
      {takesValues(condition.op) && (
        <>
          <input
            aria-label={t("work-filter-values")}
            placeholder={t("work-filter-values-hint")}
            list={suggestions.length > 0 ? `${id}-values` : undefined}
            value={text}
            onChange={(event) => {
              setText(event.target.value);
              onChange({
                ...condition,
                values: splitValues(event.target.value),
              });
            }}
          />
          {suggestions.length > 0 && (
            <datalist id={`${id}-values`}>
              {suggestions.map((value) => (
                <option key={value} value={value} />
              ))}
            </datalist>
          )}
        </>
      )}
      <button type="button" className="vl-text-button" onClick={onRemove}>
        {t("work-filter-remove")}
      </button>
    </li>
  );
}

function GroupEditor({
  group,
  path,
  statuses,
  nested,
  onChange,
}: {
  readonly group: Group;
  readonly path: Path;
  readonly statuses: readonly string[];
  readonly nested: boolean;
  readonly onChange: (path: Path, next?: FilterNode) => void;
}) {
  const t = useT();
  const add = (node: FilterNode) => {
    onChange(path, { ...group, children: [...group.children, node] });
  };
  return (
    <fieldset
      className={
        nested ? "vl-filter-group vl-filter-nested" : "vl-filter-group"
      }
    >
      <legend>
        <select
          aria-label={t("work-filter-join")}
          value={group.join}
          onChange={(event) => {
            onChange(path, {
              ...group,
              join: event.target.value === "or" ? "or" : "and",
            });
          }}
        >
          <option value="and">{t("work-filter-all")}</option>
          <option value="or">{t("work-filter-any")}</option>
        </select>
      </legend>
      <ul className="vl-filter-list">
        {group.children.map((child, index) => {
          const at = [...path, index];
          return child.kind === "condition" ? (
            <ConditionRow
              key={child.key}
              condition={child}
              statuses={statuses}
              onChange={(next) => {
                onChange(at, next);
              }}
              onRemove={() => {
                onChange(at);
              }}
            />
          ) : (
            <li key={child.key}>
              <GroupEditor
                group={child}
                path={at}
                statuses={statuses}
                nested
                onChange={onChange}
              />
              <button
                type="button"
                className="vl-text-button"
                onClick={() => {
                  onChange(at);
                }}
              >
                {t("work-filter-remove-group")}
              </button>
            </li>
          );
        })}
      </ul>
      <div className="vl-filter-actions">
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            add(newCondition());
          }}
        >
          {t("work-filter-add")}
        </button>
        {!nested && (
          <button
            type="button"
            className="vl-text-button"
            onClick={() => {
              add(
                newGroup(group.join === "and" ? "or" : "and", [newCondition()]),
              );
            }}
          >
            {t("work-filter-add-group")}
          </button>
        )}
      </div>
    </fieldset>
  );
}

/** The filter builder; `onApply` gets the edited group. */
export function FilterBuilder({
  value,
  statuses,
  onApply,
}: {
  readonly value: Group;
  readonly statuses: readonly string[];
  readonly onApply: (group: Group) => void;
}) {
  const t = useT();
  const [draft, setDraft] = useState(value);
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    onApply(draft);
  };
  return (
    <form className="vl-filter" onSubmit={submit} aria-label={t("work-filter")}>
      <GroupEditor
        group={draft}
        path={[]}
        statuses={statuses}
        nested={false}
        onChange={(path, next) => {
          setDraft((current) => {
            if (path.length > 0) return replaceAt(current, path, next);
            return next?.kind === "group" ? next : current;
          });
        }}
      />
      <div className="vl-filter-actions">
        <button type="submit" className="vl-filled-button">
          {t("work-filter-apply")}
        </button>
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setDraft(EVERYTHING);
            onApply(EVERYTHING);
          }}
        >
          {t("work-filter-clear")}
        </button>
      </div>
    </form>
  );
}
