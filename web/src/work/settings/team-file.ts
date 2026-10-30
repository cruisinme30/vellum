// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A team's settings file (`team.yaml`) as JSON, the shape
 * `GET /v1/work/teams/{team}/settings` returns and `PUT /v1/work/teams/{team}`
 * takes back. Fields the screen does not edit (labels, templates, forms,
 * `last_number`) ride along untouched.
 */

export const CATEGORIES = [
  "triage",
  "backlog",
  "unstarted",
  "started",
  "completed",
  "canceled",
  "duplicate",
] as const;

export type Category = (typeof CATEGORIES)[number];

export interface Status {
  readonly key: string;
  readonly name: string;
  readonly category: Category;
  readonly color?: string;
}

export const START_DAYS = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
] as const;

export interface CycleSettings {
  readonly duration_weeks: number;
  readonly start_day?: string;
  readonly cooldown_weeks?: number;
  readonly upcoming?: number;
  readonly auto_rollover?: boolean;
  readonly auto_add_started?: boolean;
  readonly auto_add_completed?: boolean;
}

/** A filter tree as the API stores it; see `../filter.ts`. */
export type FilterJson = Readonly<Record<string, unknown>>;

export interface TriageRule {
  readonly name: string;
  readonly when: FilterJson;
  readonly then: {
    readonly team?: string;
    readonly assignee?: string;
    readonly labels?: readonly string[];
    /** 0 none, 1 urgent … 4 low. */
    readonly priority?: number;
  };
}

export interface SlaRule {
  readonly name: string;
  readonly when: FilterJson;
  readonly within_hours: number;
}

export const SCALES = ["linear", "fibonacci", "exponential", "t-shirt"] as const;

export const GIT_MOMENTS = [
  "branch_pushed",
  "pr_opened",
  "pr_approved",
  "pr_merged",
  "pr_closed_unmerged",
] as const;

export type GitMoment = (typeof GIT_MOMENTS)[number];

export interface TeamFile {
  readonly id: string;
  readonly key: string;
  readonly name: string;
  readonly timezone?: string;
  readonly lead?: string;
  readonly members?: readonly string[];
  readonly workflow: { readonly statuses: readonly Status[]; readonly default: string };
  readonly estimates?: {
    readonly scale?: string;
    readonly allow_zero?: boolean;
    readonly unestimated_points?: number;
  };
  readonly cycles?: CycleSettings;
  readonly triage?: boolean;
  readonly triage_rules?: readonly TriageRule[];
  readonly sla?: { readonly rules?: readonly SlaRule[] };
  readonly git?: Partial<Record<GitMoment, string>>;
  readonly [other: string]: unknown;
}

/** `value` without the optional field `key`. */
export function without<T extends object>(value: T, key: keyof T & string): T {
  return Object.fromEntries(Object.entries(value).filter(([name]) => name !== key)) as T;
}

/** A status key from its name: `In Review` → `in-review`. */
export function slug(name: string): string {
  const words = name
    .toLowerCase()
    .normalize("NFKD")
    .split(/[^a-z0-9]/)
    .filter((word) => word !== "");
  return words.join("-").slice(0, 40).replace(/-$/, "");
}

/** A key from `name` not already in `taken`. */
export function freshKey(name: string, taken: readonly string[]): string {
  const base = slug(name) || "status";
  let key = base;
  let n = 2;
  while (taken.includes(key)) {
    key = `${base}-${String(n)}`;
    n += 1;
  }
  return key;
}

/** Moves a status up or down among the statuses of its own category. */
export function moveWithinCategory(
  statuses: readonly Status[],
  key: string,
  step: -1 | 1,
): Status[] {
  const list = [...statuses];
  const at = list.findIndex((status) => status.key === key);
  const current = list[at];
  if (current === undefined) return list;
  let other = at + step;
  while (other >= 0 && other < list.length && list[other]?.category !== current.category) {
    other += step;
  }
  const swap = list[other];
  if (swap === undefined) return list;
  list[other] = current;
  list[at] = swap;
  return list;
}

/** Drops Git moments that point at a status that no longer exists. */
export function pruneGit(file: TeamFile): TeamFile {
  if (file.git === undefined) return file;
  const keys = new Set(file.workflow.statuses.map((status) => status.key));
  const git = Object.fromEntries(Object.entries(file.git).filter(([, status]) => keys.has(status)));
  return { ...file, git };
}
