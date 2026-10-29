// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The demo's stand-in for the server's schema edits, CSV import and
// export, and collection blocks: just enough for every screen to work
// without a server. The server stays the authority on the rules.
import type { MockCollection } from "./mock-data";
import { resolve } from "./mock-engine";
import type {
  AddPropertyInput,
  PropertyInfo,
  QueryInput,
  SaveViewInput,
  UpdatePropertyInput,
  Value,
  ViewInfo,
  ViewResult,
} from "./types";
import { parseInput, text } from "./value";

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** Turns a view input's names into keys, as `schema.yaml` stores them. */
function keyOf(collection: MockCollection, nameOrKey: string): string {
  const property = resolve(collection, nameOrKey);
  if (!property) {
    throw new Error(`no property ${JSON.stringify(nameOrKey)}`);
  }
  return property.key;
}

/** Adds, replaces or renames a view, like `collections.save_view`. */
export function saveView(collection: MockCollection, input: SaveViewInput): MockCollection {
  const name = (input.rename_to ?? input.view).trim();
  if (name === "") {
    throw new Error("a view name needs 1 to 100 printable characters");
  }
  const at = collection.views.findIndex((v) => same(v.name, input.view));
  if (collection.views.some((v, index) => index !== at && same(v.name, name))) {
    throw new Error(`a view named ${JSON.stringify(name)} already exists`);
  }
  const optional = (key: "group" | "date" | "end", value: string | undefined) =>
    value === undefined || value === "" ? {} : { [key]: keyOf(collection, value) };
  const view: ViewInfo = {
    name,
    kind: input.kind ?? "table",
    filters: [...(input.filter ?? [])],
    sorts: [...(input.sort ?? [])],
    hidden: (input.hide ?? []).map((h) => keyOf(collection, h)),
    ...optional("group", input.group),
    ...optional("date", input.date),
    ...optional("end", input.end),
    ...(input.limit === undefined ? {} : { limit: input.limit }),
  };
  const views =
    at === -1
      ? [...collection.views, view]
      : collection.views.map((v, index) => (index === at ? view : v));
  return { ...collection, views };
}

/** Removes a view, like `collections.delete_view`. */
export function deleteView(collection: MockCollection, name: string): MockCollection {
  const views = collection.views.filter((v) => !same(v.name, name));
  if (views.length === collection.views.length) {
    throw new Error(`no view named ${JSON.stringify(name)}`);
  }
  return { ...collection, views };
}

function checkName(collection: MockCollection, name: string, except?: string): string {
  const trimmed = name.trim();
  if (trimmed === "") {
    throw new Error("a property name needs 1 to 100 printable characters");
  }
  if (collection.properties.some((p) => p.key !== except && same(p.name, trimmed))) {
    throw new Error(`a property named ${JSON.stringify(trimmed)} already exists`);
  }
  return trimmed;
}

/** A key from a name: lower-case letters, digits and `_`, unique. */
function newKey(collection: MockCollection, name: string): string {
  let slug = "";
  for (const c of name.toLowerCase()) {
    const plain = /[a-z0-9]/.test(c);
    if (plain) slug += c;
    else if (slug !== "" && !slug.endsWith("_")) slug += "_";
  }
  while (slug.endsWith("_")) slug = slug.slice(0, -1);
  if (slug !== "" && !/[a-z]/.test(slug.charAt(0))) slug = `c${slug}`;
  const base = slug.slice(0, 28) || `col_${String(collection.properties.length + 1)}`;
  const taken = new Set([
    ...collection.properties.map((p) => p.key),
    "id",
    "icon",
    "cover",
    "title",
  ]);
  let key = base;
  let n = 1;
  while (taken.has(key)) {
    n += 1;
    key = `${base}_${String(n)}`;
  }
  return key;
}

/** Adds a property at the end, like `collections.add_property`. */
export function addProperty(collection: MockCollection, input: AddPropertyInput): MockCollection {
  const name = checkName(collection, input.name);
  const computed = ["formula", "rollup", "created"].includes(input.type);
  const property: PropertyInfo = {
    key: newKey(collection, name),
    name,
    type: input.type,
    computed,
    ...(input.options?.length ? { options: input.options } : {}),
    ...(input.formula === undefined ? {} : { formula: input.formula }),
    ...(input.format === undefined ? {} : { format: input.format }),
  };
  return { ...collection, properties: [...collection.properties, property] };
}

/** Renames a property or changes its options, like `collections.update_property`. */
export function updateProperty(
  collection: MockCollection,
  input: UpdatePropertyInput,
): MockCollection {
  const old = collection.properties.find((p) => p.key === input.key);
  if (!old) {
    throw new Error(`no property with key ${JSON.stringify(input.key)}`);
  }
  const name = input.name === undefined ? old.name : checkName(collection, input.name, old.key);
  const next: PropertyInfo = {
    ...old,
    name,
    ...(input.options === undefined ? {} : { options: input.options }),
    ...(input.formula === undefined ? {} : { formula: input.formula }),
    ...(input.format === undefined ? {} : { format: input.format }),
  };
  return {
    ...collection,
    properties: collection.properties.map((p) => (p.key === old.key ? next : p)),
  };
}

/** Removes a property and the view settings that use it. */
export function deleteProperty(collection: MockCollection, key: string): MockCollection {
  const property = collection.properties.find((p) => p.key === key);
  if (!property) {
    throw new Error(`no property with key ${JSON.stringify(key)}`);
  }
  if (property.type === "title") {
    throw new Error("the title property cannot be removed");
  }
  if (property.relation?.back !== undefined) {
    throw new Error("a two-way relation is removed from both collections' schema.yaml");
  }
  const uses = (filter: string) =>
    filter.trim().toLowerCase().startsWith(property.key) ||
    filter.trim().toLowerCase().startsWith(property.name.toLowerCase());
  const keep = (value: string | undefined) => (value === key ? undefined : value);
  const views = collection.views.map((v): ViewInfo => {
    const group = keep(v.group);
    const date = keep(v.date);
    const end = keep(v.end);
    return {
      name: v.name,
      kind: v.kind,
      filters: v.filters.filter((f) => !uses(f)),
      sorts: v.sorts.filter((s) => !uses(s)),
      hidden: v.hidden.filter((h) => h !== key),
      ...(group === undefined ? {} : { group }),
      ...(date === undefined ? {} : { date }),
      ...(end === undefined ? {} : { end }),
      ...(v.limit === undefined ? {} : { limit: v.limit }),
    };
  });
  return {
    ...collection,
    properties: collection.properties.filter((p) => p.key !== key),
    views,
  };
}

const SEPARATORS = new Set([",", "\r", "\n"]);
const atSeparator = (input: string, index: number) =>
  index < input.length && SEPARATORS.has(input.charAt(index));

/** Reads one cell from `start`; returns it and where it ended. */
function readCell(input: string, start: number): [cell: string, end: number] {
  let cell = "";
  let index = start;
  if (input.charAt(index) === '"') {
    index += 1;
    while (index < input.length) {
      const c = input.charAt(index);
      if (c !== '"') {
        cell += c;
        index += 1;
      } else if (input.charAt(index + 1) === '"') {
        cell += '"';
        index += 2;
      } else {
        index += 1;
        break;
      }
    }
  }
  while (index < input.length && !atSeparator(input, index)) {
    cell += input.charAt(index);
    index += 1;
  }
  return [cell, index];
}

/** Splits CSV text into rows of cells; quoted cells may hold commas, quotes and newlines. */
export function parseCsv(input: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let index = 0;
  while (index < input.length) {
    const [cell, end] = readCell(input, index);
    row.push(cell);
    if (input.charAt(end) === ",") {
      index = end + 1;
    } else {
      rows.push(row);
      row = [];
      index = end + (input.startsWith("\r\n", end) ? 2 : 1);
    }
  }
  return rows.filter((r) => r.some((c) => c.trim() !== ""));
}

/** One CSV line; cells that need it are quoted. */
export function csvLine(cells: readonly string[]): string {
  return cells.map((c) => (/[",\r\n]/.test(c) ? `"${c.replaceAll('"', '""')}"` : c)).join(",");
}

/** A view result as CSV: the column names, then each row's shown text. */
export function csvOf(result: ViewResult): string {
  const rows = result.groups.flatMap((g) => g.rows);
  const lines = [
    csvLine(result.columns.map((c) => c.name)),
    ...rows.map((r) => csvLine(r.cells.map((c) => c.display))),
  ];
  return `${lines.join("\n")}\n`;
}

/** What a CSV file adds to a collection. */
export interface CsvRows {
  readonly rows: Record<string, Value>[];
  readonly skipped: string[];
}

/** A CSV cell as a property's value; undefined when it does not parse. */
function cellValue(property: PropertyInfo, raw: string): Value | undefined {
  if (property.type === "multi") {
    const items = raw
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s !== "");
    return { type: "list", value: items.map((item) => text(item)) };
  }
  if (property.type === "checkbox") {
    return { type: "bool", value: /^(?:true|yes|1|x)$/i.test(raw.trim()) };
  }
  const parsed = parseInput(property.type, raw);
  return "value" in parsed ? parsed.value : undefined;
}

/** Matches CSV headers to writable properties and parses each cell. */
export function csvRows(collection: MockCollection, csv: string): CsvRows {
  const [header = [], ...lines] = parseCsv(csv);
  if (header.length === 0) {
    throw new Error("the CSV file has no header row");
  }
  const columns = header.map((h) => {
    const p = resolve(collection, h);
    return p && !p.computed && p.type !== "relation" ? p : undefined;
  });
  const skipped = header.filter((_, index) => columns[index] === undefined);
  const rows = lines.map((line) => {
    const values: Record<string, Value> = {};
    for (const [index, p] of columns.entries()) {
      const value = p ? cellValue(p, line[index] ?? "") : undefined;
      if (p && value) values[p.key] = value;
    }
    return values;
  });
  return { rows, skipped };
}

/** Guesses a column's type from its non-empty cells. */
function guessType(cells: readonly string[]): AddPropertyInput["type"] {
  if (cells.length === 0) return "text";
  if (cells.every((c) => Number.isFinite(Number(c)))) return "number";
  return cells.every((c) => /^\d{4}-\d{2}-\d{2}$/.test(c)) ? "date" : "text";
}

function trimSlashes(path: string): string {
  let end = path.length;
  while (end > 0 && path.charAt(end - 1) === "/") end -= 1;
  return path.slice(0, end);
}

/** A new collection inferred from CSV: the first column is the title. */
export function collectionFromCsv(
  id: string,
  title: string,
  folder: string,
  csv: string,
): MockCollection {
  const [header = [], ...lines] = parseCsv(csv);
  if (header.length === 0) {
    throw new Error("the CSV file has no header row");
  }
  const first = header[0]?.trim() ?? "";
  const titleName = first === "" ? "Name" : first;
  let collection: MockCollection = {
    id,
    title,
    path: `${trimSlashes(folder)}/${title}.collection`,
    properties: [{ key: "name", name: titleName, type: "title", computed: false }],
    views: [{ name: "All", kind: "table", filters: [], sorts: [], hidden: [] }],
    formulas: {},
  };
  for (const [index, h] of header.slice(1).entries()) {
    const cells = lines.map((l) => (l[index + 1] ?? "").trim()).filter((c) => c !== "");
    collection = addProperty(collection, {
      collection_id: id,
      name: h.trim() === "" ? `Column ${String(index + 2)}` : h,
      type: guessType(cells),
    });
  }
  return collection;
}

/** A block's `key: value` lines; keys may repeat. */
function fenceFields(fence: string): Map<string, string[]> {
  const fields = new Map<string, string[]>();
  for (const line of fence.split("\n")) {
    const colon = line.indexOf(":");
    const value = line.slice(colon + 1).trim();
    if (colon > 0 && value !== "") {
      const key = line.slice(0, colon).trim();
      fields.set(key, [...(fields.get(key) ?? []), value]);
    }
  }
  return fields;
}

const field = (name: string, value: unknown) => (value === undefined ? {} : { [name]: value });

/** Reads a `collection` block's body into a query. */
export function embedQuery(fence: string): QueryInput {
  const fields = fenceFields(fence);
  const one = (key: string) => fields.get(key)?.[0];
  const id = one("id");
  if (id === undefined) {
    throw new Error("a collection block needs an id: line");
  }
  const limit = Number(one("limit"));
  return {
    collection_id: id,
    ...field("view", one("view")),
    ...field("kind", one("type")),
    ...field("filter", fields.get("filter")),
    ...field("sort", fields.get("sort")),
    ...field("group", one("group")),
    ...(Number.isFinite(limit) && limit > 0 ? { limit } : {}),
  };
}
