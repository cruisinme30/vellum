// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/*
 * The JSON shapes of the collections service, mirrored by hand from
 * `crates/vellum-collections` (src/service/api.rs, src/view.rs,
 * src/value.rs, src/schema.rs, src/compute.rs). Inputs keep the Rust field
 * names (snake_case); outputs are camelCase, as their serde attributes say.
 * Replace this file with the generated client once the gateway publishes
 * the JSON Schema.
 */

/** A cell value, adjacently tagged: `{"type": "number", "value": 3}`. */
export type Value =
  | { readonly type: "empty" }
  /** Text, a select option, a URL, a person handle or a row ID. */
  | { readonly type: "text"; readonly value: string }
  | { readonly type: "number"; readonly value: number }
  | { readonly type: "bool"; readonly value: boolean }
  /** `YYYY-MM-DD`, optionally with `THH:MM[:SS]`, always UTC. */
  | { readonly type: "date"; readonly value: string }
  /** Multi-select options, related row IDs, rollup `show`. */
  | { readonly type: "list"; readonly value: readonly Value[] };

/** The thirteen property types (`PropertyType::name`). */
export type PropertyType =
  | "title"
  | "text"
  | "number"
  | "select"
  | "multi"
  | "person"
  | "date"
  | "checkbox"
  | "url"
  | "relation"
  | "rollup"
  | "formula"
  | "created";

/** How a view lays out rows (`ViewKind`, snake_case). */
export type ViewKind = "table" | "board" | "gallery" | "calendar" | "timeline" | "list";

type NumberFormat = "plain" | "percent";

type Aggregation = "count" | "sum" | "avg" | "min" | "max" | "checked" | "pct" | "unique" | "show";

export interface SelectOption {
  readonly name: string;
  readonly color?: string;
}

/** `QueryInput`: runs a saved view with optional ad-hoc changes. */
export interface QueryInput {
  readonly collection_id: string;
  /** A saved view's name; the first view when absent. */
  readonly view?: string;
  readonly kind?: ViewKind;
  /** Extra filters, added to the view's (`status is Done`). */
  readonly filter?: readonly string[];
  /** Sort keys replacing the view's (`due desc`). */
  readonly sort?: readonly string[];
  readonly group?: string;
  /** Hidden properties, replacing the view's. */
  readonly hide?: readonly string[];
  readonly limit?: number;
}

/** `RowKey`. */
export interface RowKey {
  readonly collection_id: string;
  readonly row_id: string;
}

/** `CreateRowInput`. */
export interface CreateRowInput {
  readonly collection_id: string;
  /** Values by property key or name. */
  readonly values?: Readonly<Record<string, Value>>;
  readonly body?: string;
}

/** `UpdateRowInput`: an `empty` value clears a property. */
export interface UpdateRowInput {
  readonly collection_id: string;
  readonly row_id: string;
  readonly values?: Readonly<Record<string, Value>>;
  readonly body?: string;
}

/** `CollectionSummary`. */
interface CollectionSummary {
  readonly id: string;
  readonly title: string;
  /** The `name.collection` folder. */
  readonly path: string;
  readonly rows: number;
  /** Saved view names, in tab order. */
  readonly views: readonly string[];
}

/** `CollectionList`. */
export interface CollectionList {
  readonly collections: readonly CollectionSummary[];
}

/** `PropertyInfo`: what the UI needs to render headers and editors. */
export interface PropertyInfo {
  readonly key: string;
  readonly name: string;
  readonly type: PropertyType;
  /** Formula, rollup and created values are computed and read-only. */
  readonly computed: boolean;
  readonly options?: readonly SelectOption[];
  readonly relation?: { readonly to: string; readonly back?: string };
  readonly rollup?: { readonly via: string; readonly prop: string; readonly agg: Aggregation };
  readonly formula?: string;
  readonly format?: NumberFormat;
}

/** `ViewInfo`: a saved view in its `schema.yaml` text form. */
export interface ViewInfo {
  readonly name: string;
  readonly kind: ViewKind;
  readonly filters: readonly string[];
  readonly sorts: readonly string[];
  readonly group?: string;
  readonly hidden: readonly string[];
  /** Date property for calendar and timeline. */
  readonly date?: string;
  /** End-date property for timeline bars. */
  readonly end?: string;
  readonly limit?: number;
}

/** `Diagnostic`: a problem found while loading files. */
interface Diagnostic {
  readonly severity: "warning" | "error";
  readonly location: string;
  readonly message: string;
}

/** `CollectionInfo`. */
export interface CollectionInfo {
  readonly id: string;
  readonly title: string;
  readonly path: string;
  /** Properties in column order. */
  readonly properties: readonly PropertyInfo[];
  /** Saved views in tab order. */
  readonly views: readonly ViewInfo[];
  readonly diagnostics: readonly Diagnostic[];
}

/** `CellError`: why a computed cell has no value. */
interface CellError {
  readonly message: string;
  readonly span?: { readonly start: number; readonly end: number };
}

/** `Column` of a view result; hidden properties are left out. */
export interface Column {
  readonly key: string;
  readonly name: string;
  readonly type: PropertyType;
  readonly format: NumberFormat;
  readonly options?: readonly SelectOption[];
  /** False for computed properties. */
  readonly editable: boolean;
}

/** `CellView`: `value` is absent when the cell is empty or has an error. */
export interface CellView {
  readonly value?: Value;
  /** The value as text, as the server formats it. */
  readonly display: string;
  readonly error?: CellError;
}

/** `DateSpan`, for calendar and timeline views. */
export interface DateSpan {
  readonly start: string;
  readonly end: string;
}

/** `RowView`: cells line up with the result's columns. */
export interface RowView {
  readonly id: string;
  readonly title: string;
  readonly cells: readonly CellView[];
  readonly span?: DateSpan;
}

/** `Group`: `value` is null for the "No …" group (and the single ungrouped one). */
export interface Group {
  readonly value: Value | null;
  readonly label: string;
  readonly rows: readonly RowView[];
}

/** `ViewResult`: what `collections.query` returns. */
export interface ViewResult {
  readonly collection: string;
  readonly title: string;
  readonly view: string;
  readonly kind: ViewKind;
  readonly columns: readonly Column[];
  readonly groups: readonly Group[];
  /** Matching rows before the limit. */
  readonly total: number;
  readonly truncated: boolean;
}

/** `CellValue`, externally tagged. */
export type CellValue = { readonly value: Value } | { readonly error: CellError };

/** `RowDetail`: one row with every value, stored and computed. */
export interface RowDetail {
  readonly collectionId: string;
  readonly path: string;
  readonly title: string;
  readonly row: { readonly id: string; readonly cells: Readonly<Record<string, CellValue>> };
  readonly body: string;
}

interface RowReference {
  readonly collection: string;
  readonly row: string;
}

/** `RowWritten`: the result of a row write. */
export interface RowWritten {
  readonly row?: RowDetail;
  readonly changed: {
    readonly written: readonly RowReference[];
    readonly deleted: readonly RowReference[];
  };
}
