// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { CellContent } from "./cell-content";
import { propertyOf, useWriter } from "./editing";
import type { PropertyInfo, RowView, Value } from "./types";
import { bool, sameValue } from "./value";
import { ValueEditor } from "./value-editor";
import { TYPE_ICONS, uniqueRows, type ViewProperties } from "./view-properties";

type Position = readonly [row: number, column: number];

const PAGE = 10;

/** Where each navigation key moves the active cell, before clamping. */
const MOVES: Readonly<
  Record<string, (at: Position, size: Position, control: boolean) => Position>
> = {
  ArrowUp: ([r, c]) => [r - 1, c],
  ArrowDown: ([r, c]) => [r + 1, c],
  ArrowLeft: ([r, c]) => [r, c - 1],
  ArrowRight: ([r, c]) => [r, c + 1],
  PageUp: ([r, c]) => [r - PAGE, c],
  PageDown: ([r, c]) => [r + PAGE, c],
  Home: ([r], _, control) => [control ? 0 : r, 0],
  End: ([r], [rows, columns], control) => [control ? rows - 1 : r, columns - 1],
};

const clamp = (value: number, size: number) =>
  Math.min(Math.max(value, 0), size - 1);

interface CellProperties {
  readonly row: RowView;
  readonly rowIndex: number;
  readonly columnIndex: number;
  readonly property: PropertyInfo;
  readonly active: boolean;
  readonly editing: boolean;
  readonly error: string | undefined;
  readonly onSelect: () => void;
  readonly onFocus: () => void;
  readonly onEdit: () => void;
  readonly onKeyDown: (event: KeyboardEvent) => void;
  readonly onCommit: (value: Value) => void;
  readonly onCancel: () => void;
  readonly onOpen: () => void;
}

function GridCell(properties: CellProperties) {
  const t = useT();
  const { row, property, editing, error } = properties;
  const cell = row.cells[properties.columnIndex];
  const title = row.title === "" ? t("page-untitled") : row.title;
  return (
    <div
      role="gridcell"
      className="vl-grid-cell"
      aria-colindex={properties.columnIndex + 1}
      aria-readonly={property.computed || undefined}
      aria-invalid={error === undefined ? undefined : true}
      data-cell={
        String(properties.rowIndex) + `:${String(properties.columnIndex)}`
      }
      data-type={property.type}
      tabIndex={properties.active ? 0 : -1}
      onClick={properties.onSelect}
      onFocus={(event) => {
        if (event.target === event.currentTarget) {
          properties.onFocus();
        }
      }}
      onDoubleClick={properties.onEdit}
      onKeyDown={properties.onKeyDown}
    >
      {editing ? (
        <ValueEditor
          property={property}
          value={cell?.value}
          label={t("collection-edit-cell", {
            property: property.name,
            row: title,
          })}
          onCommit={properties.onCommit}
          onCancel={properties.onCancel}
        />
      ) : (
        <span className="vl-grid-value">
          <CellContent property={property} cell={cell} />
        </span>
      )}
      {property.type === "title" && !editing && (
        <button
          type="button"
          className="vl-icon-button vl-grid-open"
          tabIndex={-1}
          aria-label={t("collection-open-row", { title })}
          onClick={(event) => {
            event.stopPropagation();
            properties.onOpen();
          }}
        >
          <Icon name="open_in_new" size={18} />
        </button>
      )}
      {error !== undefined && (
        <span className="vl-cell-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}

/**
 * The table view: a full-width ARIA grid with a sticky header. Arrow keys,
 * Home, End and Page keys move between cells; Enter or F2 edits a cell
 * (Space or Enter toggles a checkbox), Enter keeps the edit, Escape drops
 * it, and Shift+Enter opens the row.
 */
export function TableView({ info, result, onOpen }: ViewProperties) {
  const t = useT();
  const hintId = useId();
  const write = useWriter(info.id);
  const rows = uniqueRows(result);
  const properties = result.columns.map((column) =>
    propertyOf(info.properties, column),
  );
  const [active, setActive] = useState<{ row: string; column: number }>();
  const [editing, setEditing] = useState(false);
  const [errors, setErrors] = useState<ReadonlyMap<string, string>>(new Map());
  const grid = useRef<HTMLDivElement>(null);
  const refocus = useRef(false);
  const rowIndex = Math.max(
    0,
    rows.findIndex((row) => row.id === active?.row),
  );
  const columnIndex = clamp(active?.column ?? 0, properties.length);

  useEffect(() => {
    if (refocus.current && !editing) {
      refocus.current = false;
      grid.current
        ?.querySelector<HTMLElement>(
          `[data-cell="${String(rowIndex)}:${String(columnIndex)}"]`,
        )
        ?.focus();
    }
  });

  const select = (r: number, c: number) => {
    const row = rows[clamp(r, rows.length)];
    if (row) {
      setActive({ row: row.id, column: clamp(c, properties.length) });
      refocus.current = true;
    }
  };
  const save = async (row: RowView, property: PropertyInfo, value: Value) => {
    setEditing(false);
    refocus.current = true;
    const key = `${row.id}:${property.key}`;
    const before = row.cells[properties.indexOf(property)]?.value;
    if (sameValue(before, value)) {
      return;
    }
    const error = await write(row.id, { [property.key]: value });
    setErrors((current) => {
      const next = new Map(current);
      if (error === undefined) {
        next.delete(key);
      } else {
        next.set(key, error);
      }
      return next;
    });
  };
  const edit = (row: RowView, property: PropertyInfo) => {
    if (property.computed) {
      return;
    }
    if (property.type === "checkbox") {
      const value = row.cells[properties.indexOf(property)]?.value;
      void save(row, property, bool(!(value?.type === "bool" && value.value)));
    } else {
      setEditing(true);
    }
  };
  const onKeyDown = (
    event: KeyboardEvent,
    row: RowView,
    property: PropertyInfo,
  ) => {
    if (editing || event.target !== event.currentTarget) {
      return;
    }
    const move = MOVES[event.key];
    const control = event.ctrlKey || event.metaKey;
    if (move) {
      event.preventDefault();
      select(
        ...move(
          [rowIndex, columnIndex],
          [rows.length, properties.length],
          control,
        ),
      );
    } else if (event.key === "Enter" && event.shiftKey) {
      event.preventDefault();
      onOpen(row.id);
    } else if (
      event.key === "Enter" ||
      event.key === "F2" ||
      (event.key === " " && property.type === "checkbox")
    ) {
      event.preventDefault();
      edit(row, property);
    }
  };

  return (
    <div className="vl-table">
      <p id={hintId} className="vl-muted vl-view-hint">
        {t("collection-table-hint")}
      </p>
      <div className="vl-grid-scroll">
        <div
          ref={grid}
          role="grid"
          className="vl-grid"
          aria-label={result.view}
          aria-describedby={hintId}
          aria-rowcount={result.total + 1}
          aria-colcount={properties.length}
          style={
            {
              "--vl-columns": String(properties.length - 1),
            } as React.CSSProperties
          }
        >
          <div
            role="row"
            className="vl-grid-row vl-grid-head"
            aria-rowindex={1}
          >
            {properties.map((property, index) => (
              <div
                key={property.key}
                role="columnheader"
                className="vl-grid-header"
                aria-colindex={index + 1}
              >
                <Icon name={TYPE_ICONS[property.type]} size={18} />
                <span>{property.name}</span>
              </div>
            ))}
          </div>
          {rows.map((row, r) => (
            <div
              key={row.id}
              role="row"
              className="vl-grid-row"
              aria-rowindex={r + 2}
            >
              {properties.map((property, c) => {
                const here = r === rowIndex && c === columnIndex;
                return (
                  <GridCell
                    key={property.key}
                    row={row}
                    rowIndex={r}
                    columnIndex={c}
                    property={property}
                    active={here}
                    editing={here && editing}
                    error={errors.get(`${row.id}:${property.key}`)}
                    onSelect={() => {
                      if (!here) {
                        setEditing(false);
                        select(r, c);
                      }
                    }}
                    onFocus={() => {
                      if (!here) {
                        setEditing(false);
                        setActive({ row: row.id, column: c });
                      }
                    }}
                    onEdit={() => {
                      select(r, c);
                      edit(row, property);
                    }}
                    onKeyDown={(event) => {
                      onKeyDown(event, row, property);
                    }}
                    onCommit={(value) => void save(row, property, value)}
                    onCancel={() => {
                      setEditing(false);
                      refocus.current = true;
                    }}
                    onOpen={() => {
                      onOpen(row.id);
                    }}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
      {rows.length === 0 && <p className="vl-muted">{t("collection-empty")}</p>}
      {result.truncated && (
        <p className="vl-muted">
          {t("collection-truncated", {
            shown: rows.length,
            total: result.total,
          })}
        </p>
      )}
    </div>
  );
}
