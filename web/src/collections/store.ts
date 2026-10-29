// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type {
  CollectionInfo,
  CollectionList,
  CreateRowInput,
  QueryInput,
  RowDetail,
  RowKey,
  RowWritten,
  UpdateRowInput,
  ViewResult,
} from "./types";

/**
 * The collections port: one method per `collections.*` operation of the
 * service (ADR 0015). The mock adapter serves the Northstar demo; the HTTP
 * adapter replaces it when the gateway client lands, without touching
 * views. Every call is asynchronous because the real one is a request; a
 * rejected promise carries the server's message.
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
  /**
   * Calls the listener after any write, so open views query again. The HTTP
   * adapter fires it after its own writes and on the server's change feed.
   */
  subscribe(listener: () => void): () => void;
}
