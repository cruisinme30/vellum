// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type {
  AddPropertyInput,
  CollectionInfo,
  CollectionList,
  CreateFromCsvInput,
  CreateRowInput,
  CsvExport,
  CsvImported,
  EmbedResult,
  ExportCsvInput,
  ImportCsvInput,
  PropertyKey,
  QueryInput,
  RowDetail,
  RowKey,
  RowWritten,
  SaveViewInput,
  UpdatePropertyInput,
  UpdateRowInput,
  ViewKey,
  ViewResult,
} from "./types";

/**
 * The collections port: one method per `collections.*` operation of the
 * service (ADR 0015, ADR 0045). The HTTP adapter talks to a Vellum server;
 * the mock adapter serves the Northstar demo when there is none. Every call
 * is asynchronous because the real one is a request; a rejected promise
 * carries the server's message.
 */
export interface CollectionsStore {
  /** `collections.list`: the collections the caller can read. */
  list(): Promise<CollectionList>;
  /** `collections.get`: schema and saved views. */
  info(collectionId: string): Promise<CollectionInfo>;
  /** `collections.query`: runs a saved view. */
  query(input: QueryInput): Promise<ViewResult>;
  /** `collections.get_row`: every value of one row, and its body. */
  getRow(key: RowKey): Promise<RowDetail>;
  createRow(input: CreateRowInput): Promise<RowWritten>;
  updateRow(input: UpdateRowInput): Promise<RowWritten>;
  /** `collections.delete_row`: moves the row to the trash and unlinks it. */
  deleteRow(key: RowKey): Promise<RowWritten>;
  /** `collections.save_view`: adds, replaces or renames a saved view. */
  saveView(input: SaveViewInput): Promise<CollectionInfo>;
  deleteView(key: ViewKey): Promise<CollectionInfo>;
  /** `collections.add_property`: a new column at the end. */
  addProperty(input: AddPropertyInput): Promise<CollectionInfo>;
  updateProperty(input: UpdatePropertyInput): Promise<CollectionInfo>;
  deleteProperty(key: PropertyKey): Promise<CollectionInfo>;
  exportCsv(input: ExportCsvInput): Promise<CsvExport>;
  /** `collections.import_csv`; `dry_run` reports without writing. */
  importCsv(input: ImportCsvInput): Promise<CsvImported>;
  /** `collections.create_from_csv`: a new collection, column types inferred. */
  createFromCsv(input: CreateFromCsvInput): Promise<CsvImported>;
  /** `collections.embed`: the rows a `collection` block in a page shows. */
  embed(fence: string): Promise<EmbedResult>;
  /**
   * Calls the listener when data may have changed, so open views query
   * again: after this tab's writes, and (HTTP) when the tab comes back into
   * view or every so often while it is visible.
   */
  subscribe(listener: () => void): () => void;
}
