// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";

import type { CollectionsStore } from "./store";
import type {
  CollectionInfo,
  CollectionList,
  CsvExport,
  CsvImported,
  EmbedResult,
  RowDetail,
  RowWritten,
  ViewResult,
} from "./types";

const segment = encodeURIComponent;
const ROOT = "/api/v1/collections";
const collection = (id: string) => `${ROOT}/${segment(id)}`;
const row = (id: string, rowId: string) => `${collection(id)}/rows/${segment(rowId)}`;
const view = (id: string, name: string) => `${collection(id)}/views/${segment(name)}`;
const property = (id: string, key: string) => `${collection(id)}/properties/${segment(key)}`;

/** How often open views reload while the tab is visible. */
export const REFRESH_MS = 30_000;
/** Reloads closer together than this are skipped (focus and visibility fire together). */
const MIN_GAP_MS = 2000;

/** Where the adapter learns that the tab is in view; tests pass fakes. */
export interface Freshness {
  readonly document?: Pick<
    Document,
    "visibilityState" | "addEventListener" | "removeEventListener"
  >;
  readonly window?: Pick<Window, "addEventListener" | "removeEventListener">;
  readonly every?: number;
  readonly now?: () => number;
}

/** The real page, when there is one. */
function browserPage(): Freshness {
  return "document" in globalThis ? { document, window: globalThis } : {};
}

/**
 * The collections port over the server's REST operations
 * (`/api/v1/collections`, ADR 0015 and ADR 0045). Views reload after this
 * tab's writes, when the tab comes back into view, and every
 * {@link REFRESH_MS} while it is visible, so edits made elsewhere show up
 * without a page reload (D191).
 */
export function createHttpCollectionsStore(
  http: Http,
  freshness: Freshness = browserPage(),
): CollectionsStore {
  const listeners = new Set<() => void>();
  const now = freshness.now ?? (() => Date.now());
  let last = now();
  const notify = () => {
    last = now();
    for (const listener of listeners) listener();
  };
  const changed = <T>(result: T): T => {
    notify();
    return result;
  };
  const visible = () => freshness.document?.visibilityState !== "hidden";
  const refresh = () => {
    if (visible() && now() - last >= MIN_GAP_MS) notify();
  };
  let timer: ReturnType<typeof setInterval> | undefined;
  const start = () => {
    freshness.document?.addEventListener("visibilitychange", refresh);
    freshness.window?.addEventListener("focus", refresh);
    timer = setInterval(refresh, freshness.every ?? REFRESH_MS);
  };
  const stop = () => {
    freshness.document?.removeEventListener("visibilitychange", refresh);
    freshness.window?.removeEventListener("focus", refresh);
    clearInterval(timer);
    timer = undefined;
  };

  return {
    list: () => http.get<CollectionList>(ROOT),
    info: (id) => http.get<CollectionInfo>(collection(id)),
    query: ({ collection_id, ...rest }) =>
      http.get<ViewResult>(`${collection(collection_id)}/query`, rest),
    getRow: ({ collection_id, row_id }) => http.get<RowDetail>(row(collection_id, row_id)),
    createRow: async (input) =>
      changed(
        await http.send<RowWritten>("POST", `${collection(input.collection_id)}/rows`, input),
      ),
    updateRow: async (input) =>
      changed(await http.send<RowWritten>("PATCH", row(input.collection_id, input.row_id), input)),
    deleteRow: async (key) =>
      changed(await http.send<RowWritten>("DELETE", row(key.collection_id, key.row_id))),
    saveView: async (input) =>
      changed(await http.send<CollectionInfo>("PUT", view(input.collection_id, input.view), input)),
    deleteView: async (key) =>
      changed(await http.send<CollectionInfo>("DELETE", view(key.collection_id, key.view))),
    addProperty: async (input) =>
      changed(
        await http.send<CollectionInfo>(
          "POST",
          `${collection(input.collection_id)}/properties`,
          input,
        ),
      ),
    updateProperty: async (input) =>
      changed(
        await http.send<CollectionInfo>("PATCH", property(input.collection_id, input.key), input),
      ),
    deleteProperty: async (key) =>
      changed(await http.send<CollectionInfo>("DELETE", property(key.collection_id, key.key))),
    exportCsv: ({ collection_id, ...rest }) =>
      http.get<CsvExport>(`${collection(collection_id)}/csv`, rest),
    importCsv: async (input) => {
      const result = await http.send<CsvImported>(
        "POST",
        `${collection(input.collection_id)}/csv`,
        input,
      );
      return result.written ? changed(result) : result;
    },
    createFromCsv: async (input) => {
      const result = await http.send<CsvImported>("POST", ROOT, input);
      return result.written ? changed(result) : result;
    },
    embed: (fence) => http.get<EmbedResult>(`${ROOT}/embed`, { fence }),
    subscribe(listener) {
      listeners.add(listener);
      if (listeners.size === 1) start();
      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) stop();
      };
    },
  };
}
