// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Where a task can be moved in My Work, and time blocks (R-MW-1, R-MW-9).
 * Dates are the person's local calendar days as the server counts them
 * (`MyWorkView.today`), so nothing here reads the browser's clock or zone.
 */

import type { SectionView } from "./api";

/**
 * A place to move a task to: a bucket name or a day, as `tasks.move` takes it.
 */
export interface MoveTarget {
  /** `inbox`, `today`, `evening`, `anytime`, `someday` or `YYYY-MM-DD`. */
  readonly to: string;
  /** The i18n key of its name, or a date to show as is. */
  readonly label: { readonly key: string } | { readonly day: string };
}

/**
 * The day `days` after `day` (`YYYY-MM-DD`), counted in UTC so no zone shifts
 * it.
 */
export function addDays(day: string, days: number): string {
  const date = new Date(`${day}T00:00:00Z`);
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
}

/** The fixed places a task can go, for the drop bar and the Move menu. */
export function moveTargets(today: string): MoveTarget[] {
  return [
    { to: "inbox", label: { key: "my-work-bucket-inbox" } },
    { to: "today", label: { key: "my-work-bucket-today" } },
    { to: "evening", label: { key: "my-work-bucket-evening" } },
    { to: addDays(today, 1), label: { key: "my-work-tomorrow" } },
    { to: addDays(today, 7), label: { key: "my-work-next-week" } },
    { to: "anytime", label: { key: "my-work-bucket-anytime" } },
    { to: "someday", label: { key: "my-work-bucket-someday" } },
  ];
}

/**
 * Where dropping on a section moves a task: its bucket, or its day for an
 * Upcoming day or week. `undefined` for Done, which completes instead.
 */
export function sectionTarget(section: SectionView): string | undefined {
  if (section.bucket === "done") return undefined;
  if (section.bucket === "upcoming")
    return section.day ?? section.week_of ?? undefined;
  return section.bucket;
}

/**
 * Only checklist items (`<doc>.<block>`) can be moved or blocked; issues and
 * comments cannot.
 */
export function isMovable(task: {
  readonly kind: string;
  readonly id: string;
}): boolean {
  return task.kind === "checklist" && task.id.includes(".");
}

/**
 * A time block: when it starts (local, `YYYY-MM-DDTHH:MM`) and how many minutes
 * it lasts.
 */
export interface TimeBlock {
  readonly start: string;
  readonly minutes: number;
}

/** The longest block the server takes: one day. */
export const MAX_BLOCK_MINUTES = 1440;

/**
 * Reads a `block` field (`2026-11-03T15:00/90`); `undefined` when absent or
 * malformed.
 */
export function parseBlock(value?: string | null): TimeBlock | undefined {
  const match = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})\/(\d{1,4})$/.exec(
    value ?? "",
  );
  if (match?.[1] === undefined || match[2] === undefined) return undefined;
  const minutes = Number(match[2]);
  if (minutes < 1 || minutes > MAX_BLOCK_MINUTES) return undefined;
  return { start: match[1], minutes };
}

/** Writes a block field; `undefined` when the start or length is not valid. */
export function formatBlock(
  start: string,
  minutes: number,
): string | undefined {
  const block = `${start.slice(0, 16)}/${String(Math.round(minutes))}`;
  return parseBlock(block) === undefined ? undefined : block;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** The block's end time of day, `HH:MM`, wrapping past midnight. */
export function blockEnd(block: TimeBlock): string {
  const [hours = 0, minutes = 0] = block.start
    .slice(11, 16)
    .split(":")
    .map(Number);
  const total = (hours * 60 + minutes + block.minutes) % 1440;
  return `${pad(Math.floor(total / 60))}:${pad(total % 60)}`;
}
