// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The people directory (`GET /api/v1/directory`, D272): who can be given
 * access, shared with or restricted to. Groups join through
 * {@link combineDirectories} with {@link createHttpGroupDirectory}.
 */

import type { Operations } from "../api/api.gen";
import type { Http } from "../api/http";

/** A person or group that access settings can name. */
export type DirectoryEntry = Operations["identity.directory"]["output"]["entries"][number];

/** Finds people and groups by part of a name, handle or email. */
export interface DirectoryApi {
  readonly search: (query: string) => Promise<DirectoryEntry[]>;
}

/** The server's people directory. */
export function createHttpDirectory(http: Http): DirectoryApi {
  return {
    async search(query) {
      const found = await http.get<Operations["identity.directory"]["output"]>(
        "/api/v1/directory",
        { q: query },
      );
      return found.entries;
    },
  };
}

/** Named groups (`GET /api/v1/directory/groups`, D275), in the same entry shape. */
export function createHttpGroupDirectory(http: Http): DirectoryApi {
  return {
    async search(query) {
      const found = await http.get<Operations["groups.search"]["output"]>(
        "/api/v1/directory/groups",
        { q: query, limit: 8 },
      );
      return found.entries;
    },
  };
}

/**
 * One directory over several sources (people, groups), keeping each
 * source's order and dropping duplicates. A source that fails is skipped
 * so people still show when groups are down.
 */
export function combineDirectories(...sources: readonly DirectoryApi[]): DirectoryApi {
  return {
    async search(query) {
      const results = await Promise.allSettled(sources.map((s) => s.search(query)));
      const seen = new Set<string>();
      return results
        .flatMap((r) => (r.status === "fulfilled" ? r.value : []))
        .filter((entry) => !seen.has(entry.principal) && seen.add(entry.principal));
    },
  };
}

/** A fixed list, for demo mode and tests. */
export function createListDirectory(entries: readonly DirectoryEntry[]): DirectoryApi {
  return {
    search(query) {
      const q = query.trim().toLowerCase();
      if (q.length < 2) return Promise.resolve([]);
      return Promise.resolve(
        entries.filter((entry) => `${entry.name} ${entry.detail}`.toLowerCase().includes(q)),
      );
    },
  };
}

/** Whether `text` is a principal typed in full: `user:<id>` or `group:<id>`. */
export const isPrincipal = (text: string) => /^(?:user|group):[\w-]+$/u.test(text.trim());
