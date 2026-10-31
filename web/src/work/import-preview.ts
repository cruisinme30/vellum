// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A quick count of the tasks in an export, shown before importing (#525).
 * The server does the real reading; this only tells the person whether
 * they picked the right file.
 */

import type { ImportFrom } from "./connections-api";

/** Counts JSON objects for which `isTask` holds, at any depth. */
function countIn(
  value: unknown,
  isTask: (object: Record<string, unknown>) => boolean,
): number {
  if (Array.isArray(value)) {
    return value.reduce<number>((sum, item) => sum + countIn(item, isTask), 0);
  }
  if (typeof value !== "object" || value === null) return 0;
  const object = value as Record<string, unknown>;
  const own = isTask(object) ? 1 : 0;
  return Object.values(object).reduce<number>(
    (sum, child) => sum + countIn(child, isTask),
    own,
  );
}

/** Google Tasks: items with a status (tasks), not the lists that hold them. */
const googleTask = (object: Record<string, unknown>) =>
  typeof object["title"] === "string" &&
  typeof object["status"] === "string" &&
  object["kind"] !== "tasks#taskList";

/** Things: to-dos and checklist items. */
const thingsTask = (object: Record<string, unknown>) =>
  object["type"] === "to-do" || object["type"] === "checklist-item";

/** Todoist's CSV template: rows whose first cell is `task`. */
function todoistTasks(content: string): number | undefined {
  const lines = content.split(/\r?\n/);
  const header = lines[0]
    ?.split(",")[0]
    ?.replaceAll('"', "")
    .trim()
    .toUpperCase();
  if (header !== "TYPE") return undefined;
  return lines
    .slice(1)
    .filter(
      (line) =>
        line.split(",")[0]?.replaceAll('"', "").trim().toLowerCase() === "task",
    ).length;
}

/**
 * How many tasks the export seems to hold; `undefined` when it is not that
 * app's format.
 */
export function previewCount(
  from: ImportFrom,
  content: string,
): number | undefined {
  if (from === "todoist") return todoistTasks(content);
  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch {
    return undefined;
  }
  return countIn(parsed, from === "google_tasks" ? googleTask : thingsTask);
}
