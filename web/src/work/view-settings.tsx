// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** A view's settings as they live in the address, and the pickers for them. */

import { useId } from "react";
import { useSearchParams } from "react-router";

import type { TeamOut, ViewIn } from "./api";
import type { GroupBy } from "./board";

export const GROUPS: readonly GroupBy[] = [
  "status",
  "assignee",
  "priority",
  "project",
  "cycle",
  "label",
  "none",
];
export const ORDERS = [
  "priority",
  "updated",
  "created",
  "due",
  "estimate",
  "title",
] as const;
export const CLOSED = ["past_week", "past_month", "all", "none"] as const;
export const VISIBILITIES = ["private", "team", "workspace"] as const;
export const CATEGORIES = [
  "triage",
  "backlog",
  "unstarted",
  "started",
  "completed",
  "canceled",
  "duplicate",
];

/** The view settings in the address. */
export interface Settings {
  readonly layout: "list" | "board";
  readonly group: GroupBy;
  readonly order: string;
  readonly closed: string;
  readonly filter: string | undefined;
  readonly view: string | undefined;
}

function pick<T extends string>(
  value: string | null,
  allowed: readonly T[],
  fallback: T,
): T {
  return value !== null && (allowed as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

export function useSettings(): [
  Settings,
  (change: Partial<Record<keyof Settings, string | undefined>>) => void,
] {
  const [parameters, setParameters] = useSearchParams();
  const settings: Settings = {
    layout: parameters.get("layout") === "board" ? "board" : "list",
    group: pick(parameters.get("group"), GROUPS, "status"),
    order: pick(parameters.get("order"), ORDERS, "priority"),
    closed: pick(parameters.get("closed"), CLOSED, "past_week"),
    filter: parameters.get("filter") ?? undefined,
    view: parameters.get("view") ?? undefined,
  };
  const change = (
    next: Partial<Record<keyof Settings, string | undefined>>,
  ) => {
    const updated = new URLSearchParams(parameters);
    for (const [key, value] of Object.entries(next)) {
      if (value === undefined) updated.delete(key);
      else updated.set(key, value);
    }
    setParameters(updated, { replace: true });
  };
  return [settings, change];
}

export function viewInput(
  team: TeamOut,
  settings: Settings,
  name: string,
  visibility: string,
): ViewIn {
  return {
    name,
    team: team.key,
    visibility,
    layout: settings.layout,
    group_by: settings.group,
    order_by: settings.order,
    show_closed: settings.closed,
    ...(settings.filter === undefined ? {} : { filter: settings.filter }),
  };
}

export function Select<T extends string>({
  label,
  value,
  options,
  name,
  onChange,
}: {
  readonly label: string;
  readonly value: T;
  readonly options: readonly T[];
  readonly name: (option: T) => string;
  readonly onChange: (value: T) => void;
}) {
  const id = useId();
  return (
    <span className="vl-work-field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={value}
        onChange={(event) => {
          onChange(event.target.value as T);
        }}
      >
        {options.map((option) => (
          <option key={option} value={option}>
            {name(option)}
          </option>
        ))}
      </select>
    </span>
  );
}
