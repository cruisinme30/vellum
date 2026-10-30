// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Search helpers shared by both stores and the views: highlighting matched
 * words, the R-DR-9 filters, and the instant title search behind ⌘K
 * (R-SR-1). Title search runs over the page tree already in memory, so it
 * answers without a network round trip, well inside the 20 ms budget.
 */

import type { Fragment, PageMeta, SearchFilters, SearchHit } from "./types";

const DAY_MS = 86_400_000;

const LEADING = new Set(['"', "'", "("]);
const TRAILING = new Set(['"', "'", ")", ",", ".", ";", ":", "!", "?"]);

/** A word without quotes and punctuation around it. */
function trimPunctuation(word: string): string {
  let start = 0;
  let end = word.length;
  while (start < end && LEADING.has(word.charAt(start))) start += 1;
  while (end > start && TRAILING.has(word.charAt(end - 1))) end -= 1;
  return word.slice(start, end);
}

/**
 * The words of a query, lower case, longest first so overlaps mark the longer
 * one.
 */
function terms(query: string): string[] {
  return [
    ...new Set(
      query
        .toLowerCase()
        .split(/\s+/)
        .map((word) => trimPunctuation(word))
        .filter((word) => word !== ""),
    ),
  ].toSorted((a, b) => b.length - a.length);
}

/**
 * Splits `text` into runs, marking every case-insensitive occurrence of a
 * query word. Joining the runs' text gives `text` back unchanged.
 */
export function highlight(text: string, query: string): Fragment[] {
  const words = terms(query);
  if (text === "" || words.length === 0)
    return text === "" ? [] : [{ text, hit: false }];
  const lower = text.toLowerCase();
  const marked = Array.from<boolean>({ length: text.length }).fill(false);
  for (const word of words) {
    for (
      let at = lower.indexOf(word);
      at !== -1;
      at = lower.indexOf(word, at + word.length)
    ) {
      marked.fill(true, at, at + word.length);
    }
  }
  const out: Fragment[] = [];
  let start = 0;
  for (let index = 1; index <= text.length; index += 1) {
    if (index === text.length || marked[index] !== marked[start]) {
      out.push({ text: text.slice(start, index), hit: marked[start] === true });
      start = index;
    }
  }
  return out;
}

/** Whether a page passes the filters. `me` is the signed-in person's handle. */
export function matchesFilters(
  page: PageMeta,
  filters: SearchFilters | undefined,
  me: string,
  now: number = Date.now(),
): boolean {
  if (filters === undefined) return true;
  if (filters.owner === "me" && page.owner !== me) return false;
  if (filters.starred === true && !page.starred) return false;
  if (filters.spaceId !== undefined && page.spaceId !== filters.spaceId)
    return false;
  if (filters.modifiedDays !== undefined) {
    const changed = Date.parse(page.updatedAt);
    if (Number.isNaN(changed) || changed < now - filters.modifiedDays * DAY_MS)
      return false;
  }
  return true;
}

/** The hits whose pages pass the filters, in their order. */
export function filterHits(
  hits: readonly SearchHit[],
  filters: SearchFilters | undefined,
  me: string,
  now: number = Date.now(),
): SearchHit[] {
  return hits.filter((hit) => matchesFilters(hit.page, filters, me, now));
}

/**
 * The earliest change time the `modifiedDays` filter lets through, in Unix
 * milliseconds.
 */
export function modifiedFrom(
  filters: SearchFilters | undefined,
  now: number = Date.now(),
) {
  return filters?.modifiedDays === undefined
    ? undefined
    : now - filters.modifiedDays * DAY_MS;
}

/**
 * How well a title matches: a prefix beats a word start, which beats a match
 * inside a word, which beats every query word somewhere. `0` is no match.
 */
function titleScore(title: string, query: string): number {
  const lower = title.toLowerCase();
  if (lower.startsWith(query)) return 4;
  const at = lower.indexOf(query);
  if (at > 0 && /[\s\-_/.:([]/.test(lower.charAt(at - 1))) return 3;
  if (at > 0) return 2;
  const words = terms(query);
  return words.length > 1 && words.every((word) => lower.includes(word))
    ? 1
    : 0;
}

function compare(a: string, b: string): number {
  if (a === b) return 0;
  return a < b ? -1 : 1;
}

/**
 * Instant title search: pages whose title matches `raw`, best match first,
 * then starred, then most recently changed. Folders are left out.
 */
export function searchTitles(
  pages: readonly PageMeta[],
  raw: string,
  limit = 8,
): PageMeta[] {
  const query = raw.trim().toLowerCase();
  if (query === "") return [];
  const found: { page: PageMeta; score: number }[] = [];
  for (const page of pages) {
    if (page.folder === true) continue;
    const score = titleScore(page.title, query);
    if (score > 0) found.push({ page, score });
  }
  // Plain comparisons: ISO times sort as text, and localeCompare is too slow
  // here.
  return found
    .toSorted(
      (a, b) =>
        b.score - a.score ||
        Number(b.page.starred) - Number(a.page.starred) ||
        compare(b.page.updatedAt, a.page.updatedAt) ||
        compare(a.page.title, b.page.title),
    )
    .slice(0, limit)
    .map((each) => each.page);
}
