// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Calendar arithmetic on `YYYY-MM-DD` days, in UTC like the server's dates.

const DAY_MS = 86_400_000;

/** Milliseconds of a day at UTC midnight. */
const timeOf = (day: string) => Date.parse(`${day}T00:00:00Z`);

/** The day `days` after `day`. */
export function addDays(day: string, days: number): string {
  return new Date(timeOf(day) + days * DAY_MS).toISOString().slice(0, 10);
}

/** Whole days from `from` to `to`. */
export function daysBetween(from: string, to: string): number {
  return Math.round((timeOf(to) - timeOf(from)) / DAY_MS);
}

/** The first day of the month a day is in, as `YYYY-MM-01`. */
export function monthOf(day: string): string {
  return `${day.slice(0, 7)}-01`;
}

/** The first day of the month `months` after a month. */
export function addMonths(month: string, months: number): string {
  const date = new Date(timeOf(month));
  date.setUTCMonth(date.getUTCMonth() + months);
  return date.toISOString().slice(0, 10);
}

/**
 * The weeks shown for a month: Monday to Sunday, from the week the month
 * starts in to the week it ends in.
 */
export function monthWeeks(month: string): string[][] {
  const weekday = (new Date(timeOf(month)).getUTCDay() + 6) % 7;
  const end = addMonths(month, 1);
  const weeks: string[][] = [];
  for (let start = addDays(month, -weekday); start < end; start = addDays(start, 7)) {
    weeks.push(Array.from({ length: 7 }, (_, index) => addDays(start, index)));
  }
  return weeks;
}

/** Today in UTC. */
export function today(now = new Date()): string {
  return now.toISOString().slice(0, 10);
}

const formatters = new Map<string, Intl.DateTimeFormat>();

/** Formats a day for people, in the browser's language. */
export function formatDay(day: string, options: Intl.DateTimeFormatOptions): string {
  const key = JSON.stringify(options);
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(undefined, { ...options, timeZone: "UTC" });
    formatters.set(key, formatter);
  }
  return formatter.format(new Date(timeOf(day)));
}
