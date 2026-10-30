// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useState } from "react";

import type { Trace } from "./api";
import { formatDuration, waterfallRows, type WaterfallRow } from "./layout";

function Details({ row }: { readonly row: WaterfallRow }) {
  return (
    <div className="vl-waterfall__details">
      {row.fields.length > 0 && (
        <dl>
          {row.fields.map(([name, value]) => (
            <div key={name}>
              <dt>{name}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {row.events.length > 0 && (
        <ul aria-label="Events">
          {row.events.map((event) => (
            <li key={`${String(event.offset_us)}-${event.message}`}>
              <span className="vl-waterfall__level">{event.level}</span> +
              {formatDuration(event.offset_us)} {event.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Row({
  row,
  open,
  onToggle,
}: {
  readonly row: WaterfallRow;
  readonly open: boolean;
  readonly onToggle: () => void;
}) {
  const label = `${row.lane === "browser" ? "Browser" : "Server"} ${row.name}, ${formatDuration(row.durationUs)}${row.error ? ", failed" : ""}`;
  return (
    <li
      className={`vl-waterfall__row vl-waterfall__row--${row.lane}${row.error ? " vl-waterfall__row--error" : ""}`}
    >
      <button
        type="button"
        aria-expanded={open}
        aria-label={label}
        onClick={onToggle}
      >
        <span
          className="vl-waterfall__name"
          style={{ paddingInlineStart: `${String(row.depth * 16)}px` }}
        >
          {row.name}
        </span>
        <span className="vl-waterfall__track" aria-hidden="true">
          <span
            className="vl-waterfall__bar"
            style={{
              left: `${row.leftPct.toFixed(2)}%`,
              width: `${row.widthPct.toFixed(2)}%`,
            }}
          />
        </span>
        <span className="vl-waterfall__time">
          {formatDuration(row.durationUs)}
        </span>
      </button>
      {open && <Details row={row} />}
    </li>
  );
}

/**
 * The span waterfall of one trace: browser spans, then server spans as a tree.
 */
export function Waterfall({ trace }: { readonly trace: Trace }) {
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const rows = waterfallRows(trace);
  const toggle = (key: string) => {
    setOpen((previous) => {
      const next = new Set(previous);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  };
  return (
    <section className="vl-waterfall" aria-label={`Trace ${trace.trace_id}`}>
      <header>
        <h2>Trace {trace.trace_id}</h2>
        <p>
          {formatDuration(trace.duration_us)} · {trace.spans.length} server
          spans
          {trace.client.length > 0 &&
            ` · ${String(trace.client.length)} browser spans`}
          {trace.dropped_spans > 0 &&
            ` · ${String(trace.dropped_spans)} spans not kept`}
        </p>
      </header>
      <ol aria-label="Spans">
        {rows.map((row) => (
          <Row
            key={row.key}
            row={row}
            open={open.has(row.key)}
            onToggle={() => {
              toggle(row.key);
            }}
          />
        ))}
      </ol>
    </section>
  );
}
