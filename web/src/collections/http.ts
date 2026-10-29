// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";

import type { CollectionsStore } from "./store";
import type { CollectionInfo, CollectionList, QueryInput, RowDetail, RowWritten } from "./types";

const segment = encodeURIComponent;
const collection = (id: string) => `/api/v1/collections/${segment(id)}`;

/**
 * The collections port over the server's REST operations
 * (`/api/v1/collections`, ADR 0015). Views refresh after this tab's writes;
 * changes made elsewhere arrive when a view loads again.
 */
export function createHttpCollectionsStore(http: Http): CollectionsStore {
  const listeners = new Set<() => void>();
  const changed = <T>(result: T): T => {
    for (const listener of listeners) listener();
    return result;
  };
  return {
    list: () => http.get<CollectionList>("/api/v1/collections"),
    info: (collectionId) => http.get<CollectionInfo>(collection(collectionId)),
    query: ({ collection_id, ...rest }: QueryInput) =>
      http.get(`${collection(collection_id)}/query`, rest),
    getRow: ({ collection_id, row_id }) =>
      http.get<RowDetail>(`${collection(collection_id)}/rows/${segment(row_id)}`),
    createRow: async (input) =>
      changed(
        await http.send<RowWritten>("POST", `${collection(input.collection_id)}/rows`, input),
      ),
    updateRow: async (input) =>
      changed(
        await http.send<RowWritten>(
          "PATCH",
          `${collection(input.collection_id)}/rows/${segment(input.row_id)}`,
          input,
        ),
      ),
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}
