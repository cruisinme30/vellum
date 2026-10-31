// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Translate } from "../../i18n/i18n";
import type { SlashItem } from "../slash/items";

/** A person or group that can be mentioned. */
interface ChipPerson {
  readonly handle: string;
  readonly name: string;
}

/** An issue offered in the link menu. */
interface ChipIssue {
  readonly key: string;
  readonly title: string;
}

/** What an issue chip shows once Work answers (#93). */
export interface IssueStatus {
  readonly title: string;
  /** The state's display name, such as "In progress". */
  readonly state: string;
  /** Whether the state counts as finished, for the strike-through. */
  readonly closed: boolean;
}

/**
 * Where the `@` and `[[` menus find people, pages and issues. The host
 * passes one object for the editor's life; its methods read the current
 * workspace each time, so the menus stay fresh without new extensions.
 */
export interface ChipSources {
  readonly people: () => readonly ChipPerson[];
  readonly pages: () => readonly {
    readonly id: string;
    readonly title: string;
  }[];
  /** Issues matching a query; absent in demo mode. */
  readonly issues?: (query: string) => Promise<readonly ChipIssue[]>;
  /** An issue's live status for its chip; absent in demo mode. */
  readonly issueStatus?: (key: string) => Promise<IssueStatus | undefined>;
  /** Opens an issue when its chip is clicked. */
  readonly openIssue?: (key: string) => void;
}

/** The most entries a menu shows. */
const LIMIT = 8;

const normalise = (text: string) => text.trim().toLowerCase();

const matches = (query: string, ...fields: string[]) => {
  const q = normalise(query);
  return q === "" || fields.some((field) => normalise(field).includes(q));
};

/** An ISO date (`YYYY-MM-DD`) in local time. */
export function isoDate(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const month = pad(date.getMonth() + 1);
  return `${String(date.getFullYear())}-${month}-${pad(date.getDate())}`;
}

const addDays = (date: Date, days: number) =>
  new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);

/** The next Monday after `date` (a week later when `date` is a Monday). */
function nextMonday(date: Date): Date {
  return addDays(date, (8 - date.getDay()) % 7 || 7);
}

const item = (
  node: string,
  id: string,
  title: string,
  description: string,
  section: string,
  icon: string,
  attributes: Record<string, string>,
): SlashItem => ({
  id,
  node,
  title,
  description,
  section,
  icon,
  attrs: attributes,
  keywords: [],
});

/**
 * Date entries for `@`: today, tomorrow, next Monday, and a typed `YYYY-MM-DD`.
 */
export function dateItems(query: string, t: Translate, now: Date): SlashItem[] {
  const section = t("chips-dates");
  const choices: [string, string, string][] = [
    ["today", t("chips-today"), "today"],
    ["tomorrow", t("chips-tomorrow"), isoDate(addDays(now, 1))],
    ["next-week", t("chips-next-monday"), isoDate(nextMonday(now))],
  ];
  const found = choices
    .filter(([key, title]) => matches(query, key, title))
    .map(([key, title, date]) =>
      item(
        "date",
        `date-${key}`,
        title,
        date === "today" ? t("chips-today-hint") : date,
        section,
        "calendar_today",
        { date },
      ),
    );
  const typed = /^\d{4}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12]\d|3[01])$/.exec(
    query.trim(),
  );
  if (typed) {
    found.unshift(
      item(
        "date",
        `date-${typed[0]}`,
        typed[0],
        "",
        section,
        "calendar_today",
        { date: typed[0] },
      ),
    );
  }
  return found;
}

/** Entries for `@`: people first, then dates. */
export function mentionItems(
  query: string,
  sources: ChipSources,
  t: Translate,
  now: Date,
): SlashItem[] {
  const section = t("chips-people");
  const people = sources
    .people()
    .filter((person) => matches(query, person.handle, person.name))
    .slice(0, LIMIT)
    .map((person) =>
      item(
        "mention",
        `person-${person.handle}`,
        person.name,
        `@${person.handle}`,
        section,
        "person",
        {
          handle: person.handle,
        },
      ),
    );
  return [...people, ...dateItems(query, t, now)];
}

/** Entries for `[[`: pages, then issues when Work is there. */
export async function linkItems(
  query: string,
  sources: ChipSources,
  t: Translate,
): Promise<SlashItem[]> {
  const pages = sources
    .pages()
    .filter((page) => page.title !== "" && matches(query, page.title))
    .slice(0, LIMIT)
    .map((page) =>
      item(
        "wikiLink",
        `page-${page.id}`,
        page.title,
        t("chips-page"),
        t("chips-pages"),
        "description",
        {
          target: page.title,
          label: "",
        },
      ),
    );
  let issues: SlashItem[] = [];
  if (sources.issues && query.trim() !== "") {
    try {
      const found = await sources.issues(query.trim());
      issues = found.slice(0, LIMIT).map((issue) =>
        item(
          "issueKey",
          `issue-${issue.key}`,
          issue.key,
          issue.title,
          t("chips-issues"),
          "check_circle",
          {
            key: issue.key,
          },
        ),
      );
    } catch {
      // Pages still show when Work is down.
    }
  }
  return [...pages, ...issues];
}
