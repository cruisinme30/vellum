// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { UpdateCache } from "./provider";

/**
 * An update cache that opens on first use. When the cache cannot open
 * (private browsing, storage blocked), the editor still works: loads come
 * back empty and appends are dropped.
 */
export function lazyCache(open: () => Promise<UpdateCache>): UpdateCache {
  let opened: Promise<UpdateCache | undefined> | undefined;
  const cache = () => {
    opened ??= open().catch((): UpdateCache | undefined => {
      return;
    });
    return opened;
  };
  return {
    async load(documentId) {
      const target = await cache();
      return (await target?.load(documentId)) ?? [];
    },
    async append(documentId, update) {
      const target = await cache();
      await target?.append(documentId, update);
    },
  };
}
