// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** A person or group that can be mentioned. */
export interface Person {
  readonly handle: string;
  readonly name: string;
}

/** A space: the shard and permission root that holds a page tree. */
export interface Space {
  readonly id: string;
  readonly slug: string;
  readonly name: string;
  readonly icon: string;
}

/** Page status from front matter. */
type PageStatus = "draft" | "in-review" | "verified" | "outdated";

/** A page as the tree and lists see it; the body lives in its collab document. */
export interface PageMeta {
  /** The page ULID, its only identity. */
  readonly id: string;
  readonly spaceId: string;
  /** `undefined` for a top-level page of the space. */
  readonly parentId: string | undefined;
  /** Sort key among siblings. */
  readonly position: number;
  readonly title: string;
  readonly icon: string;
  readonly status: PageStatus;
  readonly owner: string;
  /** ISO 8601. */
  readonly updatedAt: string;
  readonly starred: boolean;
  /** A folder of a server's drive: it holds pages and has no body. */
  readonly folder?: boolean;
}

/** Where a page goes when it moves. */
export interface MoveTarget {
  readonly spaceId: string;
  readonly parentId: string | undefined;
  /** Index among the new siblings, counted without the moved page. */
  readonly index: number;
}

/** A run of text, marked when it matched the query. */
export interface Fragment {
  readonly text: string;
  readonly hit: boolean;
}

/** A search result. */
export interface SearchHit {
  readonly page: PageMeta;
  /** Text around the match, when the body matched. */
  readonly snippet: string;
  /** The snippet split into matched and unmatched runs, when the server marked them. */
  readonly fragments?: readonly Fragment[];
}

/** Narrows a search (R-DR-9). Every field is optional; absent means "any". */
export interface SearchFilters {
  /** `me`: only pages I own. */
  readonly owner?: "me";
  /** Only pages changed in the last this many days. */
  readonly modifiedDays?: number;
  /** Only starred pages. */
  readonly starred?: boolean;
  /** Only pages in this space. */
  readonly spaceId?: string;
}
