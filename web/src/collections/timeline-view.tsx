// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { CSSProperties } from "react";

import { useT } from "../i18n/i18n";

import { addDays, daysBetween, formatDay, today } from "./dates";
import type { DateSpan, RowView } from "./types";
import { uniqueRows, type ViewProperties } from "./view-properties";

/** Days shown before the first bar and after the last. */
const MARGIN = 2;
/** The longest scale drawn, in days. */
const MAX_DAYS = 366;

/** The first day and the number of days the scale covers. */
function scaleOf(spans: readonly DateSpan[]): { first: string; days: number } {
  const starts = spans
    .map((s) => s.start)
    .toSorted((a, b) => a.localeCompare(b));
  const ends = spans.map((s) => s.end).toSorted((a, b) => b.localeCompare(a));
  const first = addDays(starts[0] ?? today(), -MARGIN);
  const last = addDays(ends[0] ?? today(), MARGIN);
  return { first, days: Math.min(daysBetween(first, last) + 1, MAX_DAYS) };
}

const short = (day: string) =>
  formatDay(day, { month: "short", day: "numeric" });

/**
 * The timeline view: each row is a bar from its date to its end date over
 * a scale of days. Rows without a date are listed after the chart.
 */
export function TimelineView({ result, onOpen }: ViewProperties) {
  const t = useT();
  const rows = uniqueRows(result);
  const dated = rows.filter(
    (row): row is RowView & { span: DateSpan } => row.span !== undefined,
  );
  const undated = rows.filter((row) => row.span === undefined);
  const { first, days } = scaleOf(dated.map((row) => row.span));
  const scale = Array.from({ length: days }, (_, index) =>
    addDays(first, index),
  );
  const now = today();
  const style = { "--vl-days": String(days) } as CSSProperties;
  const titleOf = (row: RowView) =>
    row.title === "" ? t("page-untitled") : row.title;

  return (
    <div className="vl-timeline-view">
      <div className="vl-timeline-scroll">
        <div className="vl-timeline" style={style}>
          <div className="vl-timeline-scale" aria-hidden="true">
            {scale.map((day, index) => (
              <span key={day} data-today={day === now || undefined}>
                {index === 0 || day.endsWith("-01") ? (
                  <strong>{formatDay(day, { month: "short" })}</strong>
                ) : undefined}
                {Number(day.slice(8))}
              </span>
            ))}
          </div>
          <ul className="vl-timeline-rows" aria-label={result.view}>
            {dated.map((row) => {
              const start = Math.min(
                Math.max(daysBetween(first, row.span.start), 0),
                days - 1,
              );
              const end = Math.min(daysBetween(first, row.span.end), days - 1);
              return (
                <li key={row.id} className="vl-timeline-row">
                  <button
                    type="button"
                    className="vl-timeline-bar"
                    style={{
                      gridColumn: `${String(start + 1)} / ${String(end + 2)}`,
                    }}
                    onClick={() => {
                      onOpen(row.id);
                    }}
                  >
                    <span className="vl-timeline-title">{titleOf(row)}</span>
                    <span className="vl-visually-hidden">
                      {t("collection-span", {
                        start: short(row.span.start),
                        end: short(row.span.end),
                      })}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      </div>
      {undated.length > 0 && (
        <section aria-labelledby="vl-timeline-undated">
          <h2 id="vl-timeline-undated" className="vl-list-heading">
            {t("collection-no-dates")}
          </h2>
          <ul className="vl-list">
            {undated.map((row) => (
              <li key={row.id}>
                <button
                  type="button"
                  className="vl-list-row"
                  onClick={() => {
                    onOpen(row.id);
                  }}
                >
                  {titleOf(row)}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
