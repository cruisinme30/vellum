// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** How the share dialog shows a day. */
export const DATE = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

/** The end of a picked day (`YYYY-MM-DD`) in local time, as RFC 3339. */
export function endOfDay(day: string): string | undefined {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) return undefined;
  const end = new Date(`${day}T23:59:59`);
  return Number.isNaN(end.getTime()) ? undefined : end.toISOString();
}

/** Today as `YYYY-MM-DD`, the earliest day an expiry may be. */
export function today(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const month = pad(now.getMonth() + 1);
  return `${String(now.getFullYear())}-${month}-${pad(now.getDate())}`;
}
