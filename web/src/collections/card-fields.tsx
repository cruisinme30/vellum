// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { CellContent } from "./cell-content";
import { propertyOf } from "./editing";
import type { CollectionInfo, RowView, ViewResult } from "./types";
import { isEmpty } from "./value";

interface CardFieldsProperties {
  readonly info: CollectionInfo;
  readonly result: ViewResult;
  readonly row: RowView;
  /** A property the layout already shows, such as a board's grouping. */
  readonly skip?: string | undefined;
  readonly limit?: number;
}

/** The non-empty values of a row a card shows under its title, labelled. */
export function CardFields({ info, result, row, skip, limit = 4 }: CardFieldsProperties) {
  const fields = result.columns
    .map((column, index) => ({ column, cell: row.cells[index] }))
    .filter(
      ({ column, cell }) =>
        column.type !== "title" &&
        column.key !== skip &&
        (cell?.error !== undefined || !isEmpty(cell?.value)),
    )
    .slice(0, limit);
  if (fields.length === 0) {
    return;
  }
  return (
    <span className="vl-card-fields">
      {fields.map(({ column, cell }) => (
        <span key={column.key} className="vl-card-field">
          <span className="vl-card-label">{column.name}</span>
          <span className="vl-card-value">
            <CellContent property={propertyOf(info.properties, column)} cell={cell} />
          </span>
        </span>
      ))}
    </span>
  );
}
