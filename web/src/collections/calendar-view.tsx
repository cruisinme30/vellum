// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { addMonths, formatDay, monthOf, monthWeeks, today } from "./dates";
import type { RowView } from "./types";
import { uniqueRows, type ViewProperties } from "./view-properties";

/** Rows on a day: those whose span covers it. */
const onDay = (rows: readonly RowView[], day: string) =>
  rows.filter(
    (row) =>
      row.span !== undefined && row.span.start <= day && day <= row.span.end,
  );

/**
 * The calendar view: a month grid with each row on the days its date (or
 * date span) covers, and buttons for the previous and next months.
 */
export function CalendarView({ result, onOpen }: ViewProperties) {
  const t = useT();
  const rows = uniqueRows(result);
  const dated = rows.filter((row) => row.span !== undefined);
  const [month, setMonth] = useState(() =>
    monthOf(dated[0]?.span?.start ?? today()),
  );
  const now = today();
  const label = formatDay(month, { month: "long", year: "numeric" });
  const weeks = monthWeeks(month);
  const undated = rows.length - dated.length;

  return (
    <div className="vl-calendar-view">
      <div className="vl-calendar-bar">
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("collection-previous-month")}
          onClick={() => {
            setMonth(addMonths(month, -1));
          }}
        >
          <Icon name="chevron_left" />
        </button>
        <h2 className="vl-calendar-month" aria-live="polite">
          {label}
        </h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("collection-next-month")}
          onClick={() => {
            setMonth(addMonths(month, 1));
          }}
        >
          <Icon name="chevron_right" />
        </button>
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            setMonth(monthOf(now));
          }}
        >
          {t("collection-this-month")}
        </button>
      </div>
      <table className="vl-calendar">
        <caption className="vl-visually-hidden">{label}</caption>
        <thead>
          <tr>
            {weeks[0]?.map((day) => (
              <th
                key={day}
                scope="col"
                abbr={formatDay(day, { weekday: "long" })}
              >
                {formatDay(day, { weekday: "short" })}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {weeks.map((week) => (
            <tr key={week[0]}>
              {week.map((day) => (
                <td
                  key={day}
                  data-outside={monthOf(day) !== month || undefined}
                  data-today={day === now || undefined}
                >
                  <span className="vl-calendar-day">
                    <span aria-hidden="true">{Number(day.slice(8))}</span>
                    <span className="vl-visually-hidden">
                      {formatDay(day, { day: "numeric", month: "long" })}
                    </span>
                  </span>
                  <ul className="vl-calendar-rows">
                    {onDay(dated, day).map((row) => (
                      <li key={row.id}>
                        <button
                          type="button"
                          className="vl-calendar-row"
                          onClick={() => {
                            onOpen(row.id);
                          }}
                        >
                          {row.title === "" ? t("page-untitled") : row.title}
                        </button>
                      </li>
                    ))}
                  </ul>
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {undated > 0 && (
        <p className="vl-muted">
          {t("collection-undated", { count: undated })}
        </p>
      )}
    </div>
  );
}
