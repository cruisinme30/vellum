// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Small labels for an issue's priority, SLA and state, shared by the list,
 * board, triage inbox and My Issues. Colour is never the only signal: each
 * chip spells out its meaning.
 */

import { useT } from "../i18n/i18n";

import type { IssueOut, SlaOut, TeamOut } from "./api";

/** Priority names by number, as filters and group keys spell them. */
export const PRIORITIES = ["none", "urgent", "high", "normal", "low"] as const;

/** The filter name of a priority number (`1` is `urgent`). */
function priorityName(priority: number): (typeof PRIORITIES)[number] {
  return PRIORITIES[priority] ?? "none";
}

/** An issue's state as its team names it. */
export function stateName(team: TeamOut | undefined, state: string): string {
  return team?.statuses.find((status) => status.key === state)?.name ?? state;
}

/** The priority, or nothing when none is set. */
function PriorityChip({ priority }: { readonly priority: number }) {
  const t = useT();
  const name = priorityName(priority);
  if (name === "none") return;
  return (
    <span className={`vl-work-chip vl-work-chip-priority vl-priority-${name}`}>
      {t(`work-priority-${name}`)}
    </span>
  );
}

const RISKS = new Set(["gray", "yellow", "orange", "red"]);

/**
 * The SLA clock: gray with time to spare, yellow past half, orange past
 * three quarters, red once breached. Hidden once the clock stopped.
 */
function SlaChip({ sla }: { readonly sla: SlaOut | null | undefined }) {
  const t = useT();
  if (
    sla === undefined ||
    sla === null ||
    (sla.stopped_at ?? undefined) !== undefined
  )
    return;
  const risk = RISKS.has(sla.risk) ? sla.risk : "gray";
  const due = new Date(sla.breaches_at);
  const when = Number.isNaN(due.getTime())
    ? sla.breaches_at
    : due.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
      });
  return (
    <span
      className={`vl-work-chip vl-sla vl-sla-${risk}`}
      title={sla.breaches_at}
    >
      {t(`work-sla-${risk}`, { when })}
    </span>
  );
}

/** The row of chips an issue shows in lists and on cards. */
export function IssueChips({ issue }: { readonly issue: IssueOut }) {
  return (
    <span className="vl-work-chips">
      <PriorityChip priority={issue.priority} />
      <SlaChip sla={issue.sla} />
      {issue.estimate !== undefined && issue.estimate !== null && (
        <span className="vl-work-chip">{issue.estimate}</span>
      )}
      {issue.assignee !== undefined && issue.assignee !== null && (
        <span className="vl-work-chip vl-work-chip-person">
          @{issue.assignee}
        </span>
      )}
    </span>
  );
}
