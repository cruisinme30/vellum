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

export type Aggregation =
  "count" | "sum" | "avg" | "min" | "max" | "checked" | "pct" | "unique" | "show";

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

/** `CellView`: `value` is absent only when the cell has an error; empty is `{"type": "empty"}`. */
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

/**
 * `Group`: `value` is null for the "No …" group and for the single group of an
 * ungrouped view (label ""); tell them apart by the view having a group property.
 */
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

/** `ViewKey`: names one saved view (any case). */
export interface ViewKey {
  readonly collection_id: string;
  readonly view: string;
}

/** `SaveViewInput`: replaces the view named `view`, or adds it at the end. */
export interface SaveViewInput {
  readonly collection_id: string;
  readonly view: string;
  readonly rename_to?: string;
  readonly kind?: ViewKind;
  /** Filters in `schema.yaml` text (`Status is Done`), all of which must match. */
  readonly filter?: readonly string[];
  /** Sort keys (`Due desc`), most significant first. */
  readonly sort?: readonly string[];
  readonly group?: string;
  readonly hide?: readonly string[];
  readonly date?: string;
  readonly end?: string;
  readonly limit?: number;
}

/** The types a property can be added as: anything but a second title. */
export type NewPropertyType = Exclude<PropertyType, "title">;

/** `AddPropertyInput`: the server picks the key from the name. */
export interface AddPropertyInput {
  readonly collection_id: string;
  readonly name: string;
  readonly type: NewPropertyType;
  readonly options?: readonly SelectOption[];
  readonly formula?: string;
  readonly via?: string;
  readonly prop?: string;
  readonly agg?: Aggregation;
  readonly format?: NumberFormat;
  /** A relation's target collection; the target gets the paired property. */
  readonly to?: string;
  /** The paired property's name; this collection's title when absent. */
  readonly back_name?: string;
}

/**
 * `UpdatePropertyInput`: fields left out stay as they are. A rename
 * rewrites the formulas that name the property.
 */
export interface UpdatePropertyInput {
  readonly collection_id: string;
  readonly key: string;
  readonly name?: string;
  /** A new type; titles and relations keep theirs. */
  readonly type?: Exclude<PropertyType, "title" | "relation">;
  readonly via?: string;
  readonly prop?: string;
  readonly agg?: Aggregation;
  /** The full new option list. */
  readonly options?: readonly SelectOption[];
  readonly formula?: string;
  readonly format?: NumberFormat;
}

/** `PropertyKey`. */
export interface PropertyKey {
  readonly collection_id: string;
  readonly key: string;
}

/** `ExportCsvInput`: a view's columns, filters and order, every row. */
export interface ExportCsvInput {
  readonly collection_id: string;
  readonly view?: string;
}

/** `CsvExport`. */
export interface CsvExport {
  /** A suggested file name (`Issues - Open.csv`). */
  readonly fileName: string;
  readonly csv: string;
}

/** `ImportCsvInput`: adds rows, all or nothing. */
export interface ImportCsvInput {
  readonly collection_id: string;
  readonly csv: string;
  /** Check the file and report, without writing. */
  readonly dry_run?: boolean;
}

/** `CreateFromCsvInput`: a new `Title.collection` folder in a drive folder. */
export interface CreateFromCsvInput {
  readonly folder: string;
  readonly title: string;
  readonly csv: string;
  readonly dry_run?: boolean;
}

/** `CsvImported`: what an import did, or would do on a dry run. */
export interface CsvImported {
  readonly written: boolean;
  readonly rows: number;
  /** Headers that matched no writable property. */
  readonly skippedColumns: readonly string[];
  readonly diagnostics: readonly Diagnostic[];
  readonly collection: CollectionInfo;
  readonly changed: RowWritten["changed"];
}

/** `EmbedResult`: a `collection` block's view, as the reader sees it. */
export interface EmbedResult {
  readonly collectionId: string;
  readonly title: string;
  /** The `name.collection` folder. */
  readonly path: string;
  readonly result: ViewResult;
}
