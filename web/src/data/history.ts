// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";

/** Who wrote a version, as the drive records it (`ActorView`). */
export type VersionAuthor =
  | { readonly type: "user"; readonly user: string }
  | { readonly type: "model"; readonly on_behalf_of: string }
  | { readonly type: "external" }
  | { readonly type: "system" };

/** One saved version of a page. */
export interface PageVersion {
  readonly id: string;
  /** When it was written, read from the version's ULID. */
  readonly savedAt: Date;
  /** Its size in bytes. */
  readonly size: number;
  readonly author: VersionAuthor;
}

/**
 * The saved versions of pages (issue #89). Only a Vellum server keeps
 * them; demo mode has none, and the app hides history there.
 */
export interface PageHistory {
  /** A page's versions, newest first. */
  list(pageId: string): Promise<PageVersion[]>;
  /** A version's Markdown, or `undefined` when it is not text. */
  read(pageId: string, versionId: string): Promise<string | undefined>;
  /** Makes an old version the page's content again, as a new version. */
  restore(pageId: string, versionId: string): Promise<void>;
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * The time a ULID was made: its first ten characters are the Unix time in
 * milliseconds, in Crockford base 32. `undefined` for anything else.
 */
export function ulidTime(id: string): Date | undefined {
  if (id.length !== 26) {
    return undefined;
  }
  let ms = 0;
  for (const char of id.slice(0, 10).toUpperCase()) {
    const digit = CROCKFORD.indexOf(char);
    if (digit === -1) {
      return undefined;
    }
    ms = ms * 32 + digit;
  }
  return new Date(ms);
}

interface VersionView {
  readonly id: string;
  readonly size: number;
  readonly author: VersionAuthor;
}

const item = (id: string) => `/api/v1/drive/items/${encodeURIComponent(id)}`;

/** Page history over the drive's version operations (`/api/v1/drive`). */
export function createHttpPageHistory(http: Http): PageHistory {
  return {
    async list(pageId) {
      const { versions } = await http.get<{ versions: VersionView[] }>(
        `${item(pageId)}/versions`,
      );
      return versions
        .map((version) => ({
          id: version.id,
          savedAt: ulidTime(version.id) ?? new Date(0),
          size: version.size,
          author: version.author,
        }))
        .toSorted(
          (a, b) =>
            b.savedAt.getTime() - a.savedAt.getTime() ||
            b.id.localeCompare(a.id),
        );
    },
    async read(pageId, versionId) {
      const version = await http.get<{ content?: string | null }>(
        `${item(pageId)}/versions/${encodeURIComponent(versionId)}`,
      );
      return version.content ?? undefined;
    },
    async restore(pageId, versionId) {
      await http.send(
        "POST",
        `${item(pageId)}/versions/${encodeURIComponent(versionId)}/restore`,
      );
    },
  };
}
