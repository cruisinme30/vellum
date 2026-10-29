// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { CellValue, PropertyType, Value } from "./types";

export const EMPTY: Value = { type: "empty" };

export const text = (value: string): Value => ({ type: "text", value });
export const number = (value: number): Value => ({ type: "number", value });
export const bool = (value: boolean): Value => ({ type: "bool", value });
export const date = (value: string): Value => ({ type: "date", value });
export const list = (values: readonly Value[]): Value => ({ type: "list", value: values });

/** Whether a value is blank: nothing, empty text or a list of blanks. */
export function isEmpty(value: Value | undefined): boolean {
  if (value === undefined || value.type === "empty") {
    return true;
  }
  if (value.type === "text") {
    return value.value === "";
  }
  return value.type === "list" && value.value.every((item) => isEmpty(item));
}

/** The text items of a value: a list's texts, or a text on its own. */
export function texts(value: Value | undefined): string[] {
  if (value?.type === "list") {
    return value.value.flatMap((item) => texts(item));
  }
  return value?.type === "text" && value.value !== "" ? [value.value] : [];
}

/** Formats a number without float noise, as the server does. */
export function formatNumber(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 1e10) / 1e10);
}

/** A value as plain text (lists joined with commas). */
export function plain(value: Value | undefined): string {
  switch (value?.type) {
    case "text":
    case "date": {
      return value.value;
    }
    case "number": {
      return formatNumber(value.value);
    }
    case "bool": {
      return value.value ? "Yes" : "No";
    }
    case "list": {
      return value.value
        .filter((item) => !isEmpty(item))
        .map((item) => plain(item))
        .join(", ");
    }
    default: {
      return "";
    }
  }
}

/** The calendar day of a date value (`YYYY-MM-DD`), or "" for anything else. */
export function dayOf(value: Value | undefined): string {
  return value?.type === "date" ? value.value.slice(0, 10) : "";
}

/** The value of a row-detail cell, or undefined when it holds an error. */
export function cellValue(cell: CellValue | undefined): Value | undefined {
  return cell !== undefined && "value" in cell ? cell.value : undefined;
}

/** What went wrong while parsing typed input: a message id. */
type ParseResult = { readonly value: Value } | { readonly error: string };

/** Parses what someone typed into an editor for a property type. */
export function parseInput(type: PropertyType, raw: string): ParseResult {
  const input = raw.trim();
  if (input === "") {
    return { value: type === "title" ? text("") : EMPTY };
  }
  if (type === "number") {
    const n = Number(input.replaceAll(",", ""));
    return Number.isFinite(n) ? { value: number(n) } : { error: "collection-error-number" };
  }
  if (type === "date") {
    return /^\d{4}-\d{2}-\d{2}$/.test(input) && !Number.isNaN(Date.parse(input))
      ? { value: date(input) }
      : { error: "collection-error-date" };
  }
  if (type === "url" && !URL.canParse(input)) {
    return { error: "collection-error-url" };
  }
  return { value: text(input) };
}

/** The text an editor starts from. */
export function inputText(value: Value | undefined): string {
  return value?.type === "number" ? formatNumber(value.value) : dayOrPlain(value);
}

function dayOrPlain(value: Value | undefined): string {
  return value?.type === "date" ? dayOf(value) : plain(value);
}

/** Whether two values hold the same data. */
export function sameValue(a: Value | undefined, b: Value | undefined): boolean {
  return JSON.stringify(a ?? EMPTY) === JSON.stringify(b ?? EMPTY);
}
