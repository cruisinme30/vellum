// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useRef, useState, type DragEvent, type KeyboardEvent } from "react";
import { useNavigate } from "react-router";

import { useT } from "../i18n/i18n";

import type { Place } from "./api";
import { dropPlace, isWithin, placeFor, type OutlineMove, type OutlineRow } from "./outline";

const KEY_MOVES: Readonly<Record<string, OutlineMove>> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowRight: "indent",
  ArrowLeft: "outdent",
};

const ZONES: Partial<Record<Place["at"], Drop["zone"]>> = { before: "before", after: "after" };

const DRAG_TYPE = "application/x-vellum-section";

interface BookTreeProperties {
  readonly book: string;
  readonly rows: readonly OutlineRow[];
  readonly current: string | undefined;
  readonly maxDepth: number;
  /** Moves a section; resolves once the book has reloaded. */
  readonly onMove: (section: string, place: Place) => Promise<void>;
  readonly sectionPath: (section: string) => string;
}

interface Drop {
  readonly id: string;
  readonly zone: "before" | "inside" | "after";
}

/**
 * The outline an editor rearranges: a WAI-ARIA tree with one tab stop.
 * Drag a section onto the top or bottom edge of another to put it before
 * or after, or onto its middle to nest it. Alt+Shift+arrows do the same
 * from the keyboard (WCAG 2.5.7): up and down among siblings, right to
 * nest under the section above, left to move out.
 */
export function BookTree({
  book,
  rows,
  current,
  maxDepth,
  onMove,
  sectionPath,
}: BookTreeProperties) {
  const t = useT();
  const navigate = useNavigate();
  const [focusId, setFocusId] = useState<string>();
  const [dragging, setDragging] = useState<string>();
  const [drop, setDrop] = useState<Drop>();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const tabStop = rows.find((r) => r.id === (focusId ?? current))?.id ?? rows[0]?.id;

  const focus = (id: string | undefined) => {
    if (id === undefined) return;
    setFocusId(id);
    list.current?.querySelector<HTMLElement>(`[data-section-id="${CSS.escape(id)}"]`)?.focus();
  };

  const move = (id: string, place: Place | undefined, title: string) => {
    if (place === undefined || busy) {
      if (place === undefined) setStatus(t("books-move-blocked", { title }));
      return;
    }
    setBusy(true);
    void onMove(id, place).then(() => {
      setBusy(false);
      setStatus(t("books-moved", { title }));
      requestAnimationFrame(() => {
        focus(id);
      });
    });
  };

  const onKeyDown = (event: KeyboardEvent, row: OutlineRow, at: number) => {
    const reorder = event.altKey && event.shiftKey ? KEY_MOVES[event.key] : undefined;
    if (reorder) {
      event.preventDefault();
      move(row.id, placeFor(rows, row.id, reorder, maxDepth), row.node.title);
      return;
    }
    const actions: Record<string, () => void> = {
      ArrowDown: () => {
        focus(rows[at + 1]?.id);
      },
      ArrowUp: () => {
        focus(rows[at - 1]?.id);
      },
      Home: () => {
        focus(rows[0]?.id);
      },
      End: () => {
        focus(rows.at(-1)?.id);
      },
      ArrowLeft: () => {
        focus(row.parent);
      },
      Enter: () => {
        void navigate(sectionPath(row.id));
      },
    };
    const action = actions[event.key];
    if (action) {
      event.preventDefault();
      action();
    }
  };

  const zoneOf = (event: DragEvent<HTMLElement>, row: OutlineRow): Drop | undefined => {
    const dragged = rows.find((r) => r.id === dragging);
    if (dragged === undefined || isWithin(rows, row.id, dragged.id)) return undefined;
    const box = event.currentTarget.getBoundingClientRect();
    const place = dropPlace(row, dragged, event.clientY - box.top, box.height, maxDepth);
    if (place === undefined) return undefined;
    const zone = ZONES[place.at] ?? "inside";
    return { id: row.id, zone };
  };

  // A tree may hold only tree items (axe aria-required-children), so the
  // help text and the live status sit beside it.
  return (
    <>
      <p id={`vl-book-tree-help-${book}`} className="vl-visually-hidden">
        {t("books-tree-help")}
      </p>
      <div
        className="vl-book-tree"
        role="tree"
        aria-label={t("books-contents")}
        aria-busy={busy}
        aria-describedby={`vl-book-tree-help-${book}`}
        ref={list}
      >
        {rows.map((row, at) => {
          const label =
            row.node.number == undefined ? row.node.title : `${row.node.number} ${row.node.title}`;
          const zone = drop?.id === row.id ? drop.zone : undefined;
          return (
            <div
              key={row.id}
              role="treeitem"
              data-section-id={row.id}
              data-drop={zone}
              aria-level={row.level}
              aria-setsize={row.siblings.length}
              aria-posinset={row.index + 1}
              aria-selected={row.id === current}
              aria-current={row.id === current ? "page" : undefined}
              tabIndex={row.id === tabStop ? 0 : -1}
              className="vl-book-tree-row"
              style={{ paddingInlineStart: `${String(row.level * 14 - 6)}px` }}
              draggable={!busy}
              onDragStart={(event) => {
                setDragging(row.id);
                event.dataTransfer.setData(DRAG_TYPE, row.id);
                event.dataTransfer.effectAllowed = "move";
              }}
              onDragEnd={() => {
                setDragging(undefined);
                setDrop(undefined);
              }}
              onDragOver={(event) => {
                const next = zoneOf(event, row);
                if (next === undefined) {
                  setDrop(undefined);
                  return;
                }
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (next.id !== drop?.id || next.zone !== drop.zone) setDrop(next);
              }}
              onDragLeave={() => {
                if (drop?.id === row.id) setDrop(undefined);
              }}
              onDrop={(event) => {
                event.preventDefault();
                const dragged = rows.find((r) => r.id === dragging);
                const box = event.currentTarget.getBoundingClientRect();
                if (dragged !== undefined && !isWithin(rows, row.id, dragged.id)) {
                  move(
                    dragged.id,
                    dropPlace(row, dragged, event.clientY - box.top, box.height, maxDepth),
                    dragged.node.title,
                  );
                }
                setDragging(undefined);
                setDrop(undefined);
              }}
              onFocus={(event) => {
                if (event.target === event.currentTarget) setFocusId(row.id);
              }}
              onKeyDown={(event) => {
                if (event.target === event.currentTarget) onKeyDown(event, row, at);
              }}
              onClick={() => {
                void navigate(sectionPath(row.id));
              }}
            >
              <span className="vl-book-tree-grip" aria-hidden="true">
                ⋮⋮
              </span>
              <span className="vl-book-tree-title">{label}</span>
            </div>
          );
        })}
      </div>
      <p className="vl-visually-hidden" role="status">
        {status}
      </p>
    </>
  );
}
