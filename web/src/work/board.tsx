// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A view's issues as a grouped list or a board. On a board grouped by
 * status or priority, cards move between columns by dragging or with
 * Shift+← / Shift+→; J/K (and H/L on a board) move the focus.
 */

import { useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { Link } from "react-router";

import { useT } from "../i18n/i18n";

import type { GroupOut, IssueOut, TeamOut } from "./api";
import { IssueChips, PRIORITIES, stateName } from "./chips";

/** What issues are grouped by. */
export type GroupBy =
  "status" | "assignee" | "priority" | "project" | "cycle" | "label" | "team" | "none";

/** Grouping where a card can move between columns. */
function canMove(groupBy: GroupBy): boolean {
  return groupBy === "status" || groupBy === "priority";
}

/** A group's heading. */
function useGroupName(team: TeamOut | undefined, groupBy: GroupBy) {
  const t = useT();
  return (key: string | null | undefined): string => {
    if (groupBy === "none") return t("work-group-all");
    if (key === undefined || key === null)
      return t("work-group-none", { field: t(`work-group-${groupBy}`) });
    switch (groupBy) {
      case "status": {
        return stateName(team, key);
      }
      case "priority": {
        return (PRIORITIES as readonly string[]).includes(key) ? t(`work-priority-${key}`) : key;
      }
      case "assignee": {
        return `@${key}`;
      }
      default: {
        return key;
      }
    }
  };
}

const TYPING = new Set(["INPUT", "SELECT", "TEXTAREA"]);

function cards(root: HTMLElement | null | undefined): HTMLAnchorElement[] {
  return root === null || root === undefined
    ? []
    : [...root.querySelectorAll<HTMLAnchorElement>("a[data-issue]")];
}

function columnOf(card: Element): HTMLElement | null {
  return card.closest<HTMLElement>("[data-column]");
}

/** The card after (or before) `from`, or the first card. */
function nextInList(all: readonly HTMLElement[], from: HTMLElement, step: -1 | 1) {
  const at = all.indexOf(from);
  return at === -1 ? all[0] : all[at + step];
}

/** The card in the nearest non-empty column to the right or left, at the same row or the last. */
function nextAcross(
  root: HTMLElement | null,
  from: HTMLElement,
  step: -1 | 1,
): HTMLElement | undefined {
  const column = columnOf(from);
  if (root === null || column === null) return undefined;
  const columns = [...root.querySelectorAll<HTMLElement>("[data-column]")];
  const at = columns.indexOf(column);
  const ahead = step === 1 ? columns.slice(at + 1) : columns.slice(0, at).toReversed();
  const row = Math.max(0, cards(column).indexOf(from as HTMLAnchorElement));
  const target = ahead.map((each) => cards(each)).find((inColumn) => inColumn.length > 0);
  return target?.[Math.min(row, target.length - 1)];
}

/** Where J/K (and H/L on a board) move the focus from `from`. */
function focusTarget(
  key: string,
  layout: "list" | "board",
  root: HTMLElement | null,
  from: HTMLElement,
): HTMLElement | undefined {
  switch (key) {
    case "j": {
      return nextInList(cards(root), from, 1);
    }
    case "k": {
      return nextInList(cards(root), from, -1);
    }
    case "l": {
      return layout === "board" ? nextAcross(root, from, 1) : undefined;
    }
    case "h": {
      return layout === "board" ? nextAcross(root, from, -1) : undefined;
    }
    default: {
      return undefined;
    }
  }
}

function IssueLink({
  issue,
  draggable,
  onKeyMove,
}: {
  readonly issue: IssueOut;
  readonly draggable: boolean;
  readonly onKeyMove?: ((issue: IssueOut, step: -1 | 1) => void) | undefined;
}) {
  return (
    <Link
      to={`/issues/${encodeURIComponent(issue.key)}`}
      data-issue={issue.key}
      draggable={draggable}
      onDragStart={(event: DragEvent) => {
        event.dataTransfer.setData("text/x-vellum-issue", issue.key);
        event.dataTransfer.effectAllowed = "move";
      }}
      onKeyDown={(event: KeyboardEvent) => {
        if (onKeyMove === undefined || !event.shiftKey) return;
        if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
          event.preventDefault();
          onKeyMove(issue, event.key === "ArrowRight" ? 1 : -1);
        }
      }}
    >
      <span className="vl-issue-key">{issue.key}</span>
      <span className="vl-issue-title">{issue.title}</span>
      <IssueChips issue={issue} />
    </Link>
  );
}

/** The issues of a view. */
export function IssueResults({
  groups,
  team,
  layout,
  groupBy,
  onMove,
}: {
  readonly groups: readonly GroupOut[];
  readonly team: TeamOut | undefined;
  readonly layout: "list" | "board";
  readonly groupBy: GroupBy;
  /** Moves an issue into a group; absent when cards cannot move. */
  readonly onMove?: (issue: IssueOut, group: string) => Promise<void>;
}) {
  const t = useT();
  const root = useRef<HTMLDivElement>(null);
  const [announce, setAnnounce] = useState("");
  const [over, setOver] = useState<string>();
  const name = useGroupName(team, groupBy);
  const movable = layout === "board" && onMove !== undefined && canMove(groupBy);
  const total = groups.reduce((sum, group) => sum + group.issues.length, 0);

  const move = (issue: IssueOut, key: string | null | undefined) => {
    if (onMove === undefined || key === undefined || key === null) return;
    onMove(issue, key).then(
      () => {
        setAnnounce(t("work-moved", { key: issue.key, group: name(key) }));
        // Keep the focus on the moved card once the board redraws.
        requestAnimationFrame(() => {
          root.current
            ?.querySelector<HTMLElement>(`a[data-issue="${CSS.escape(issue.key)}"]`)
            ?.focus();
        });
      },
      () => {
        setAnnounce(t("work-move-failed", { key: issue.key }));
      },
    );
  };

  const keyMove = (issue: IssueOut, step: -1 | 1) => {
    const at = groups.findIndex((group) => group.issues.some((each) => each.id === issue.id));
    const target = groups[at + step];
    if (target !== undefined) move(issue, target.key);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    const modified = event.altKey || event.ctrlKey || event.metaKey || event.shiftKey;
    if (TYPING.has(target.tagName) || modified) return;
    const found = focusTarget(event.key, layout, root.current, target);
    if (found !== undefined) {
      event.preventDefault();
      found.focus();
    }
  };

  if (total === 0 && layout === "list") {
    return <p className="vl-muted">{t("work-view-empty")}</p>;
  }

  return (
    // The container only forwards J/K/H/L to the cards inside it.
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions
    <div ref={root} className={`vl-results vl-results-${layout}`} onKeyDown={onKeyDown}>
      <p className="vl-visually-hidden" role="status" aria-live="polite">
        {announce}
      </p>
      {groups.map((group) => {
        const key = group.key ?? undefined;
        const heading = name(group.key);
        const dropProperties = movable
          ? {
              onDragOver: (event: DragEvent) => {
                event.preventDefault();
                setOver(key);
              },
              onDragLeave: () => {
                setOver(undefined);
              },
              onDrop: (event: DragEvent) => {
                event.preventDefault();
                setOver(undefined);
                const dropped = event.dataTransfer.getData("text/x-vellum-issue");
                const issue = groups.flatMap((g) => g.issues).find((each) => each.key === dropped);
                if (issue !== undefined && !group.issues.some((each) => each.id === issue.id)) {
                  move(issue, group.key);
                }
              },
            }
          : {};
        if (layout === "list" && group.issues.length === 0) return;
        return (
          <section
            key={key ?? "none"}
            className={over === key && movable ? "vl-group vl-group-over" : "vl-group"}
            data-column={key ?? ""}
            aria-label={heading}
            {...dropProperties}
          >
            {(groupBy !== "none" || layout === "board") && (
              <h2 className="vl-group-heading">
                {heading} <span className="vl-muted">{group.issues.length}</span>
              </h2>
            )}
            <ul className="vl-issue-list">
              {group.issues.map((issue) => (
                <li key={`${issue.id}-${key ?? ""}`}>
                  <IssueLink
                    issue={issue}
                    draggable={movable}
                    onKeyMove={movable ? keyMove : undefined}
                  />
                </li>
              ))}
            </ul>
          </section>
        );
      })}
    </div>
  );
}
