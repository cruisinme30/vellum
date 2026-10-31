// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// View filters as the server reads them (view.rs): `status is not Done`,
// `est > 3`, `owner is empty`. Properties are named by key or name.
import type { MockCollection } from "./mock-data";
import { itemTexts, resolve, type MockWorld } from "./mock-engine";
import type { PropertyInfo, Value } from "./types";
import { dayOf, inputText, isEmpty } from "./value";

interface Filter {
  readonly key: string;
  readonly op: string;
  readonly operand: string;
}

const UNARY = ["is not empty", "is empty"];
const WORDS = ["not contains", "contains", "is not", "is", "not"];
const SYMBOLS = ["!=", "==", "<=", ">=", "=", "<", ">"];

/** The earliest operator in a filter; the longest one wins a tie. */
function firstOperator(lower: string) {
  let best: { at: number; op: string; length: number } | undefined;
  const candidates = [
    ...WORDS.map((op) => ({
      at: lower.indexOf(` ${op} `),
      op,
      length: op.length + 2,
    })),
    ...SYMBOLS.map((op) => ({ at: lower.indexOf(op), op, length: op.length })),
  ];
  for (const candidate of candidates) {
    const better =
      best === undefined ||
      candidate.at < best.at ||
      (candidate.at === best.at && candidate.length > best.length);
    if (candidate.at > 0 && better) {
      best = candidate;
    }
  }
  return best;
}

/** Parses one filter against a collection's properties. */
export function parseFilter(collection: MockCollection, raw: string): Filter {
  const source = raw.trim();
  // A trailing space lets `owner is` read as "is (nothing)", as on the server.
  const lower = `${source.toLowerCase()} `;
  const unary = UNARY.find((op) => lower.endsWith(` ${op} `));
  const found =
    unary === undefined
      ? firstOperator(lower)
      : {
          at: source.length - unary.length - 1,
          op: unary,
          length: unary.length + 1,
        };
  const property = found && resolve(collection, source.slice(0, found.at));
  if (!found || !property) {
    throw new Error(`cannot read filter ${JSON.stringify(raw)}`);
  }
  return {
    key: property.key,
    op: found.op === "==" || found.op === "=" ? "is" : found.op,
    operand: source.slice(found.at + found.length).trim(),
  };
}

function compareTo(
  value: Value | undefined,
  operand: string,
): number | undefined {
  if (value?.type === "number") {
    return value.value - Number(operand);
  }
  if (value?.type === "date") {
    return dayOf(value).localeCompare(operand);
  }
  return value?.type === "text"
    ? value.value.toLowerCase().localeCompare(operand.toLowerCase())
    : undefined;
}

function equals(
  property: PropertyInfo,
  items: readonly string[],
  value: Value | undefined,
  operand: string,
): boolean {
  if (operand === "") {
    return isEmpty(value);
  }
  if (property.type === "checkbox") {
    const checked = value?.type === "bool" && value.value;
    return (
      ["true", "yes", "checked"].includes(operand.toLowerCase()) === checked
    );
  }
  const own =
    value?.type === "number" || value?.type === "date"
      ? [inputText(value)]
      : items;
  return own.some((item) => item.toLowerCase() === operand.toLowerCase());
}

interface Subject {
  readonly property: PropertyInfo;
  readonly value: Value | undefined;
  readonly items: readonly string[];
  readonly operand: string;
}

const contains = ({ items, operand }: Subject) =>
  items.some((item) => item.toLowerCase().includes(operand.toLowerCase()));
const ordered = (test: (order: number) => boolean) => (subject: Subject) => {
  const order = compareTo(subject.value, subject.operand);
  return order !== undefined && test(order);
};
const isEqual = (s: Subject) => equals(s.property, s.items, s.value, s.operand);

const TESTS: Readonly<Record<string, (subject: Subject) => boolean>> = {
  "is empty": ({ value }) => isEmpty(value),
  "is not empty": ({ value }) => !isEmpty(value),
  contains,
  "not contains": (subject) => !contains(subject),
  is: isEqual,
  "is not": (subject) => !isEqual(subject),
  not: (subject) => !isEqual(subject),
  "!=": (subject) => !isEqual(subject),
  "<": ordered((order) => order < 0),
  "<=": ordered((order) => order <= 0),
  ">": ordered((order) => order > 0),
  ">=": ordered((order) => order >= 0),
};

/** Whether a value passes one filter. */
export function matches(
  world: MockWorld,
  property: PropertyInfo,
  value: Value | undefined,
  filter: Filter,
): boolean {
  const items = itemTexts(world, property, value);
  return (
    TESTS[filter.op]?.({ property, value, items, operand: filter.operand }) ??
    false
  );
}
