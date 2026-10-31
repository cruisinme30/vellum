// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Today's date as `YYYY-MM-DD` in the user's time zone. */
export function formatToday(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const month = pad(now.getMonth() + 1);
  return `${String(now.getFullYear())}-${month}-${pad(now.getDate())}`;
}
