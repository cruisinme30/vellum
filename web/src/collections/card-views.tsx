// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useT } from "../i18n/i18n";

import { CardFields } from "./card-fields";
import { uniqueRows, type ViewProperties } from "./view-properties";

/** The gallery view: a grid of cards, each opening its row. */
export function GalleryView({ info, result, onOpen }: ViewProperties) {
  const t = useT();
  const rows = uniqueRows(result);
  if (rows.length === 0) {
    return <p className="vl-muted">{t("collection-empty")}</p>;
  }
  return (
    <ul className="vl-gallery" aria-label={result.view}>
      {rows.map((row) => (
        <li key={row.id}>
          <button
            type="button"
            className="vl-card vl-gallery-card"
            onClick={() => {
              onOpen(row.id);
            }}
          >
            <span className="vl-card-cover" aria-hidden="true">
              {(row.title === "" ? "?" : row.title).slice(0, 1).toUpperCase()}
            </span>
            <span className="vl-card-title">
              {row.title === "" ? t("page-untitled") : row.title}
            </span>
            <CardFields info={info} result={result} row={row} />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** The list view: compact rows, grouped when the view groups. */
export function ListView({ info, result, onOpen }: ViewProperties) {
  const t = useT();
  if (uniqueRows(result).length === 0) {
    return <p className="vl-muted">{t("collection-empty")}</p>;
  }
  return (
    <div className="vl-list-view">
      {result.groups.map((group) => (
        <section key={group.label} aria-label={group.label === "" ? result.view : group.label}>
          {group.label !== "" && <h2 className="vl-list-heading">{group.label}</h2>}
          <ul className="vl-list">
            {group.rows.map((row) => (
              <li key={row.id}>
                <button
                  type="button"
                  className="vl-list-row"
                  onClick={() => {
                    onOpen(row.id);
                  }}
                >
                  <span className="vl-card-title">
                    {row.title === "" ? t("page-untitled") : row.title}
                  </span>
                  <CardFields info={info} result={result} row={row} limit={3} />
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
