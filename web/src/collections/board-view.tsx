// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import { useT } from "../i18n/i18n";

import { CardFields } from "./card-fields";
import { groupValue, useWriter } from "./editing";
import { MoveMenu } from "./move-menu";
import type { Group, RowView } from "./types";
import type { ViewProperties } from "./view-properties";

const DRAG_TYPE = "application/x-vellum-row";

/**
 * Makes an element a drop zone for dragged cards. Native listeners, since
 * a column is a region, not a control; the keyboard path is the card's
 * "Move to" menu and Alt+arrow keys.
 */
function useDropZone(onDrop: (rowId: string) => void) {
  const [over, setOver] = useState(false);
  const cleanup = useRef<() => void>(undefined);
  const attach = useCallback(
    (element: HTMLElement | null) => {
      cleanup.current?.();
      cleanup.current = undefined;
      if (!element) {
        return;
      }
      const dragOver = (event: DragEvent) => {
        if (event.dataTransfer?.types.includes(DRAG_TYPE)) {
          event.preventDefault();
          setOver(true);
        }
      };
      const leave = () => {
        setOver(false);
      };
      const drop = (event: DragEvent) => {
        event.preventDefault();
        setOver(false);
        const id = event.dataTransfer?.getData(DRAG_TYPE) ?? "";
        if (id !== "") {
          onDrop(id);
        }
      };
      element.addEventListener("dragover", dragOver);
      element.addEventListener("dragleave", leave);
      element.addEventListener("drop", drop);
      cleanup.current = () => {
        element.removeEventListener("dragover", dragOver);
        element.removeEventListener("dragleave", leave);
        element.removeEventListener("drop", drop);
      };
    },
    [onDrop],
  );
  return { attach, over };
}

interface ColumnProperties {
  readonly group: Group;
  readonly index: number;
  readonly groups: readonly Group[];
  readonly movable: boolean;
  readonly hintId: string;
  readonly view: ViewProperties;
  readonly onMove: (row: RowView, to: number) => void;
}

function BoardColumn({
  group,
  index,
  groups,
  movable,
  hintId,
  view,
  onMove,
}: ColumnProperties) {
  const t = useT();
  const headingId = useId();
  const onDrop = useCallback(
    (rowId: string) => {
      const row = groups.flatMap((g) => g.rows).find((r) => r.id === rowId);
      if (row && !group.rows.some((r) => r.id === rowId)) {
        onMove(row, index);
      }
    },
    [groups, group, index, onMove],
  );
  const { attach, over } = useDropZone(onDrop);
  const onKeyDown = (event: KeyboardEvent, row: RowView) => {
    const step = { ArrowLeft: -1, ArrowRight: 1 }[event.key];
    if (event.altKey && step !== undefined && movable) {
      event.preventDefault();
      const to = index + step;
      if (to >= 0 && to < groups.length) {
        onMove(row, to);
      }
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      view.onOpen(row.id);
    }
  };
  return (
    <section
      ref={attach}
      className="vl-board-column"
      data-column={index}
      aria-labelledby={headingId}
      data-over={over || undefined}
    >
      <h2 id={headingId} className="vl-board-heading">
        <span className="vl-chip" data-color={optionColor(view, group)}>
          {group.label}
        </span>{" "}
        <span className="vl-muted">{group.rows.length}</span>
      </h2>
      <ul className="vl-board-cards">
        {group.rows.map((row) => (
          <li key={row.id} className="vl-board-item">
            <div
              role="button"
              tabIndex={0}
              className="vl-card"
              data-row-id={row.id}
              draggable={movable}
              aria-describedby={movable ? hintId : undefined}
              onClick={() => {
                view.onOpen(row.id);
              }}
              onKeyDown={(event) => {
                onKeyDown(event, row);
              }}
              onDragStart={(event) => {
                event.dataTransfer.setData(DRAG_TYPE, row.id);
                event.dataTransfer.effectAllowed = "move";
              }}
            >
              <span className="vl-card-title">
                {row.title === "" ? t("page-untitled") : row.title}
              </span>
              <CardFields
                info={view.info}
                result={view.result}
                row={row}
                skip={groupKey(view)}
              />
            </div>
            {movable && (
              <MoveMenu
                title={row.title === "" ? t("page-untitled") : row.title}
                targets={groups.map((g) => g.label)}
                current={index}
                onMove={(to) => {
                  onMove(row, to);
                }}
              />
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function groupKey({ info, result }: ViewProperties): string | undefined {
  return info.views.find((v) => v.name === result.view)?.group;
}

function optionColor(view: ViewProperties, group: Group): string | undefined {
  const property = view.info.properties.find((p) => p.key === groupKey(view));
  return property?.options?.find((o) => o.name === group.label)?.color;
}

/**
 * The board view: one column per group of the query. Cards move between
 * columns by dragging, with Alt+Left and Alt+Right, or with their
 * "Move to" menu; moving one sets the grouping property.
 */
export function BoardView(view: ViewProperties) {
  const t = useT();
  const hintId = useId();
  const { info, result } = view;
  const write = useWriter(info.id);
  const property = info.properties.find((p) => p.key === groupKey(view));
  const movable = property !== undefined && !property.computed;
  const [status, setStatus] = useState("");
  // A moved card is a new element in its new column; focus follows it there.
  const focusCard = useRef<{ id: string; column: number }>(undefined);
  const board = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const target = focusCard.current;
    const card =
      target &&
      board.current?.querySelector<HTMLElement>(
        `[data-column="${String(target.column)}"] [data-row-id="${target.id}"]`,
      );
    if (card) {
      focusCard.current = undefined;
      card.focus();
    }
  });

  const onMove = useCallback(
    (row: RowView, to: number) => {
      const group = result.groups[to];
      if (!property || !group) {
        return;
      }
      focusCard.current = { id: row.id, column: to };
      void write(row.id, {
        [property.key]: groupValue(property.type, group.value),
      }).then((error) => {
        if (error !== undefined) {
          focusCard.current = undefined;
        }
        setStatus(
          error ??
            t("collection-moved", { title: row.title, group: group.label }),
        );
      });
    },
    [result.groups, property, write, t],
  );

  return (
    <div className="vl-board-view">
      {movable && (
        <p id={hintId} className="vl-muted vl-view-hint">
          {t("collection-board-hint")}
        </p>
      )}
      <div ref={board} className="vl-board">
        {result.groups.map((group, index) => (
          <BoardColumn
            key={group.label}
            group={group}
            index={index}
            groups={result.groups}
            movable={movable}
            hintId={hintId}
            view={view}
            onMove={onMove}
          />
        ))}
      </div>
      <p className="vl-visually-hidden" role="status">
        {status}
      </p>
    </div>
  );
}
