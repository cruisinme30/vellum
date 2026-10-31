// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Translate } from "../i18n/i18n";

import type { ActorView } from "./api";

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;
const BYTES_FORMAT = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 1,
});
const WHEN = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

/** A byte count as a short size: `0 B`, `512 B`, `1.5 MB`. */
export function formatBytes(bytes: number): string {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${BYTES_FORMAT.format(value)} ${UNITS[unit] ?? "B"}`;
}

/**
 * A server timestamp as a local date and time; the text itself when it does not
 * parse.
 */
export function formatWhen(timestamp: string): string {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? timestamp : WHEN.format(date);
}

/** Who made a change, from the signed-in person's point of view. */
export function actorText(
  t: Translate,
  actor: ActorView,
  me: string | undefined,
): string {
  switch (actor.type) {
    case "user": {
      return actor.user === me ? t("drive-actor-you") : t("drive-actor-person");
    }
    case "model": {
      return actor.on_behalf_of === me
        ? t("drive-actor-model-you")
        : t("drive-actor-model");
    }
    case "external": {
      return t("drive-actor-external");
    }
    default: {
      return t("drive-actor-system");
    }
  }
}

/** The share of a quota used, 0 to 1; undefined without a quota. */
export function usedShare(
  used: number,
  quota: number | null | undefined,
): number | undefined {
  if (quota === undefined || quota === null || quota <= 0) return undefined;
  return Math.min(1, used / quota);
}
