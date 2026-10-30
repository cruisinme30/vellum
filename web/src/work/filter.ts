// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The Work filter tree as the builder edits it, and its JSON form, which is
 * what the server's `filter` parameter and saved views store:
 * `{"and":[{"field":"priority","op":"is","values":["urgent"]},{"or":[…]}]}`.
 */

/** One condition. */
export interface Condition {
  readonly kind: "condition";
  /** Keeps the builder's rows stable; not sent to the server. */
  readonly key: string;
  readonly field: Field;
  readonly op: Op;
  readonly values: readonly string[];
}

/** Conditions joined by AND or OR. */
export interface Group {
  readonly kind: "group";
  /** Keeps the builder's rows stable; not sent to the server. */
  readonly key: string;
  readonly join: "and" | "or";
  readonly children: readonly (Condition | Group)[];
}

export type FilterNode = Condition | Group;

export const FIELDS = [
  "status",
  "category",
  "priority",
  "assignee",
  "creator",
  "subscriber",
  "label",
  "project",
  "milestone",
  "cycle",
  "team",
  "parent",
  "estimate",
  "due",
  "created",
  "updated",
  "started",
  "completed",
  "title",
  "text",
  "blocked",
  "sla",
  "repeat",
] as const;

export type Field = (typeof FIELDS)[number];

export type Op =
  | "is"
  | "is_not"
  | "includes_all"
  | "is_empty"
  | "is_not_empty"
  | "contains"
  | "not_contains"
  | "before"
  | "after"
  | "on_or_before"
  | "on_or_after"
  | "lt"
  | "gt"
  | "lte"
  | "gte";

const DATES = new Set<Field>([
  "due",
  "created",
  "updated",
  "started",
  "completed",
]);
const TEXTS = new Set<Field>(["title", "text"]);

/** The operators that suit a field, the first being the default. */
export function opsFor(field: Field): readonly Op[] {
  if (DATES.has(field))
    return [
      "before",
      "after",
      "on_or_before",
      "on_or_after",
      "is_empty",
      "is_not_empty",
    ];
  if (TEXTS.has(field)) return ["contains", "not_contains"];
  if (field === "estimate")
    return [
      "is",
      "is_not",
      "lt",
      "gt",
      "lte",
      "gte",
      "is_empty",
      "is_not_empty",
    ];
  if (field === "blocked" || field === "sla" || field === "category")
    return ["is", "is_not"];
  if (field === "label")
    return ["is", "is_not", "includes_all", "is_empty", "is_not_empty"];
  return ["is", "is_not", "is_empty", "is_not_empty"];
}

/** Whether an operator takes values. */
export function takesValues(op: Op): boolean {
  return op !== "is_empty" && op !== "is_not_empty";
}

/** Suggested values for a field; free text is allowed too. */
export function suggestionsFor(
  field: Field,
  statuses: readonly string[],
): readonly string[] {
  switch (field) {
    case "priority": {
      return ["urgent", "high", "normal", "low", "none"];
    }
    case "category": {
      return [
        "triage",
        "backlog",
        "unstarted",
        "started",
        "completed",
        "canceled",
        "duplicate",
      ];
    }
    case "sla": {
      return ["gray", "yellow", "orange", "red"];
    }
    case "blocked": {
      return ["true", "false"];
    }
    case "status": {
      return statuses;
    }
    case "assignee":
    case "creator":
    case "subscriber": {
      return ["@me"];
    }
    case "cycle": {
      return ["@current"];
    }
    case "repeat": {
      return ["1d", "1w", "2w", "1m"];
    }
    default: {
      return DATES.has(field)
        ? ["@today", "@today-7d", "@today+7d", "@today+2w"]
        : [];
    }
  }
}

let made = 0;

/** A fresh key for a builder row. */
function rowKey(): string {
  made += 1;
  return `f${String(made)}`;
}

/** A new condition: assigned to me. */
export function newCondition(field: Field = "assignee"): Condition {
  return {
    kind: "condition",
    key: rowKey(),
    field,
    op: opsFor(field)[0] ?? "is",
    values: field === "assignee" ? ["@me"] : [],
  };
}

/** A new group. */
export function newGroup(
  join: "and" | "or",
  children: readonly FilterNode[] = [],
): Group {
  return { kind: "group", key: rowKey(), join, children };
}

/** The empty filter: every issue. */
export const EVERYTHING: Group = {
  kind: "group",
  key: "root",
  join: "and",
  children: [],
};

type Json = Record<string, unknown>;

function toJson(node: FilterNode): Json {
  if (node.kind === "condition") {
    return {
      field: node.field,
      op: node.op,
      values: takesValues(node.op) ? node.values : [],
    };
  }
  return { [node.join]: node.children.map((child) => toJson(child)) };
}

/** Drops conditions still missing values, and groups left empty. */
function complete(node: FilterNode): FilterNode | undefined {
  if (node.kind === "condition") {
    return takesValues(node.op) && node.values.length === 0 ? undefined : node;
  }
  const children = node.children
    .map((child) => complete(child))
    .filter((c) => c !== undefined);
  return children.length === 0 ? undefined : { ...node, children };
}

/** The filter's JSON text for the server, or `undefined` for "every issue". */
export function serialize(node: FilterNode): string | undefined {
  const kept = complete(node);
  return kept === undefined ? undefined : JSON.stringify(toJson(kept));
}

const isRecord = (value: unknown): value is Json =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function fromJson(value: unknown): FilterNode | undefined {
  if (!isRecord(value)) return undefined;
  for (const join of ["and", "or"] as const) {
    const list = value[join];
    if (Array.isArray(list)) {
      const children = list.map((child) => fromJson(child));
      if (children.includes(undefined)) return undefined;
      return newGroup(join, children as FilterNode[]);
    }
  }
  const { field, op, values } = value;
  if (
    typeof field === "string" &&
    (FIELDS as readonly string[]).includes(field) &&
    typeof op === "string" &&
    opsFor(field as Field).includes(op as Op)
  ) {
    const list = Array.isArray(values)
      ? values.filter((v) => typeof v === "string")
      : [];
    return {
      kind: "condition",
      key: rowKey(),
      field: field as Field,
      op: op as Op,
      values: list,
    };
  }
  return undefined;
}

/**
 * Reads the server's JSON text into a group the builder can edit, or
 * `undefined` when the text uses something the builder does not show
 * (such as `not`).
 */
export function parse(text: string | undefined | null): Group | undefined {
  if (text === undefined || text === null || text.trim() === "")
    return EVERYTHING;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return undefined;
  }
  const node = fromJson(value);
  if (node === undefined) return undefined;
  return node.kind === "group" ? node : newGroup("and", [node]);
}

/** Splits typed values on commas. */
export function splitValues(text: string): string[] {
  return text
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value !== "");
}
