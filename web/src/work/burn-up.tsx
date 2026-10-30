// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A cycle's burn-up: scope and completed points per day, as an SVG chart
 * with a table of the same numbers for screen readers and copying.
 */

import { useId } from "react";

import { useT } from "../i18n/i18n";

/** One day of a burn-up. */
export interface BurnPoint {
  readonly date: string;
  readonly scope: number;
  readonly completed: number;
}

const WIDTH = 560;
const HEIGHT = 220;
const PAD = { top: 12, right: 12, bottom: 28, left: 36 };

function path(
  points: readonly BurnPoint[],
  value: (p: BurnPoint) => number,
  x: (index: number) => number,
  y: (v: number) => number,
) {
  return points
    .map(
      (p, index) =>
        `${index === 0 ? "M" : "L"}${x(index).toFixed(1)},${y(value(p)).toFixed(1)}`,
    )
    .join(" ");
}

/** Short date for the axis: `Oct 29`. */
function day(date: string): string {
  const parsed = new Date(`${date}T00:00:00Z`);
  return Number.isNaN(parsed.getTime())
    ? date
    : parsed.toLocaleDateString(undefined, {
        month: "short",
        day: "numeric",
        timeZone: "UTC",
      });
}

/** The burn-up chart. */
export function BurnUpChart({
  points,
  title,
}: {
  readonly points: readonly BurnPoint[];
  readonly title: string;
}) {
  const t = useT();
  const id = useId();
  if (points.length === 0)
    return <p className="vl-muted">{t("work-burn-empty")}</p>;
  const top = Math.max(1, ...points.map((p) => Math.max(p.scope, p.completed)));
  const inner = {
    w: WIDTH - PAD.left - PAD.right,
    h: HEIGHT - PAD.top - PAD.bottom,
  };
  const x = (index: number) =>
    PAD.left +
    (points.length === 1
      ? inner.w / 2
      : (index / (points.length - 1)) * inner.w);
  const y = (v: number) => PAD.top + inner.h - (v / top) * inner.h;
  const ticks = [0, Math.round(top / 2), top];
  const first = points[0];
  const last = points.at(-1);
  return (
    <figure className="vl-burn-up">
      <svg
        viewBox={`0 0 ${String(WIDTH)} ${String(HEIGHT)}`}
        role="img"
        aria-labelledby={`${id}-title`}
      >
        <title id={`${id}-title`}>
          {t("work-burn-summary", {
            title,
            completed: last?.completed ?? 0,
            scope: last?.scope ?? 0,
          })}
        </title>
        {ticks.map((tick) => (
          <g key={tick}>
            <line
              className="vl-burn-grid"
              x1={PAD.left}
              x2={WIDTH - PAD.right}
              y1={y(tick)}
              y2={y(tick)}
            />
            <text
              className="vl-burn-axis"
              x={PAD.left - 6}
              y={y(tick)}
              textAnchor="end"
              dominantBaseline="middle"
            >
              {tick}
            </text>
          </g>
        ))}
        {first !== undefined && (
          <text
            className="vl-burn-axis"
            x={x(0)}
            y={HEIGHT - 8}
            textAnchor="start"
          >
            {day(first.date)}
          </text>
        )}
        {last !== undefined && points.length > 1 && (
          <text
            className="vl-burn-axis"
            x={x(points.length - 1)}
            y={HEIGHT - 8}
            textAnchor="end"
          >
            {day(last.date)}
          </text>
        )}
        <path
          className="vl-burn-scope"
          d={path(points, (p) => p.scope, x, y)}
        />
        <path
          className="vl-burn-completed"
          d={path(points, (p) => p.completed, x, y)}
        />
        {points.map((p, index) => (
          <circle
            key={p.date}
            className="vl-burn-dot"
            cx={x(index)}
            cy={y(p.completed)}
            r={3}
          />
        ))}
      </svg>
      <figcaption className="vl-burn-legend">
        <span className="vl-burn-key vl-burn-key-scope">
          {t("work-burn-scope")}
        </span>
        <span className="vl-burn-key vl-burn-key-completed">
          {t("work-burn-completed")}
        </span>
      </figcaption>
      <details className="vl-burn-table">
        <summary>{t("work-burn-table")}</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">{t("work-burn-date")}</th>
              <th scope="col">{t("work-burn-scope")}</th>
              <th scope="col">{t("work-burn-completed")}</th>
            </tr>
          </thead>
          <tbody>
            {points.map((p) => (
              <tr key={p.date}>
                <th scope="row">{p.date}</th>
                <td>{p.scope}</td>
                <td>{p.completed}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
