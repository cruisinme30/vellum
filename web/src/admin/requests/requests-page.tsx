// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState, type ComponentProps } from "react";

import {
  AdminApiError,
  fetchRequests,
  fetchTrace,
  type RequestFilter,
  type Trace,
  type TraceSummary,
} from "./api";
import { formatDuration } from "./layout";
import { Waterfall } from "./waterfall";

import "./requests.css";

/** Loading state of an async value. */
type Load<T> =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: T }
  | { readonly state: "failed"; readonly message: string };

/** A loaded value tagged with the input it was loaded for. */
type Keyed<K, T> = { readonly key: K; readonly load: Load<T> } | undefined;

/** The load for `key`, or loading while the value for an older key is shown. */
function current<K, T>(keyed: Keyed<K, T>, key: K): Load<T> {
  return keyed?.key === key ? keyed.load : { state: "loading" };
}

function failure(error: unknown): { state: "failed"; message: string } {
  if (error instanceof AdminApiError) {
    return { state: "failed", message: `${error.message} (request ${error.requestId})` };
  }
  return { state: "failed", message: String(error) };
}

function Filters({ onApply }: { readonly onApply: (filter: RequestFilter) => void }) {
  const submit: NonNullable<ComponentProps<"form">["onSubmit"]> = (event) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const text = (name: string) => {
      const value = data.get(name);
      return typeof value === "string" ? value.trim() : "";
    };
    const minimum = Number.parseInt(text("min"), 10);
    onApply({
      ...(text("route") === "" ? {} : { route: text("route") }),
      ...(text("id") === "" ? {} : { id: text("id") }),
      ...(Number.isNaN(minimum) ? {} : { minDurationMs: minimum }),
      errorsOnly: data.get("errors") === "on",
    });
  };
  return (
    <form className="vl-requests__filters" onSubmit={submit} aria-label="Filter requests">
      <label>
        Route <input name="route" placeholder="/api/v1/pages/{page_id}" />
      </label>
      <label>
        Request or trace id <input name="id" />
      </label>
      <label>
        Slower than (ms) <input name="min" inputMode="numeric" />
      </label>
      <label>
        <input type="checkbox" name="errors" /> Errors only
      </label>
      <button type="submit">Apply</button>
    </form>
  );
}

function RequestTable({
  rows,
  selected,
  onSelect,
}: {
  readonly rows: readonly TraceSummary[];
  readonly selected: string | undefined;
  readonly onSelect: (traceId: string) => void;
}) {
  if (rows.length === 0) {
    return <p>No requests match. The buffer keeps the most recent 10,000.</p>;
  }
  return (
    // The table scrolls sideways when large text or zoom makes it wider than a phone,
    // so the scroller takes keyboard focus (WCAG 2.1.1, axe scrollable-region-focusable).
    // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- see above
    <div className="vl-requests__scroll" role="region" aria-label="Requests table" tabIndex={0}>
      <table className="vl-requests__table">
        <caption>Recent requests, newest first</caption>
        <thead>
          <tr>
            <th scope="col">Time</th>
            <th scope="col">Request</th>
            <th scope="col">Status</th>
            <th scope="col">Duration</th>
            <th scope="col">Spans</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.seq}
              aria-selected={row.trace_id === selected}
              className={row.error ? "vl-requests__row--error" : undefined}
            >
              <td>{new Date(row.started_at_ms).toLocaleTimeString()}</td>
              <td>
                <button
                  type="button"
                  onClick={() => {
                    onSelect(row.trace_id);
                  }}
                >
                  {row.method ?? ""} {row.route ?? row.name}
                </button>
              </td>
              <td>{row.status ?? (row.error ? "failed" : "")}</td>
              <td>{formatDuration(row.duration_us)}</td>
              <td>
                {row.span_count}
                {row.has_client ? " + browser" : ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Admin → Requests: recent requests and the waterfall of the one selected. */
export function RequestsPage({ fetcher }: { readonly fetcher?: typeof fetch }) {
  const [filter, setFilter] = useState<RequestFilter>({});
  const [list, setList] = useState<Keyed<RequestFilter, TraceSummary[]>>();
  const [selected, setSelected] = useState<string | undefined>();
  const [trace, setTrace] = useState<Keyed<string, Trace>>();

  useEffect(() => {
    let live = true;
    fetchRequests(filter, fetcher).then(
      (value) => {
        if (live) setList({ key: filter, load: { state: "ready", value } });
      },
      (error: unknown) => {
        if (live) setList({ key: filter, load: failure(error) });
      },
    );
    return () => {
      live = false;
    };
  }, [filter, fetcher]);

  useEffect(() => {
    if (selected === undefined) return;
    let live = true;
    fetchTrace(selected, fetcher).then(
      (value) => {
        if (live) setTrace({ key: selected, load: { state: "ready", value } });
      },
      (error: unknown) => {
        if (live) setTrace({ key: selected, load: failure(error) });
      },
    );
    return () => {
      live = false;
    };
  }, [selected, fetcher]);

  const rows = current(list, filter);
  const detail = selected === undefined ? undefined : current(trace, selected);
  return (
    <section className="vl-requests">
      <h1>Requests</h1>
      <Filters onApply={setFilter} />
      {rows.state === "loading" && <p role="status">Loading requests…</p>}
      {rows.state === "failed" && <p role="alert">Could not load requests: {rows.message}</p>}
      {rows.state === "ready" && (
        <RequestTable rows={rows.value} selected={selected} onSelect={setSelected} />
      )}
      {detail?.state === "loading" && <p role="status">Loading trace…</p>}
      {detail?.state === "failed" && <p role="alert">Could not load the trace: {detail.message}</p>}
      {detail?.state === "ready" && <Waterfall trace={detail.value} />}
    </section>
  );
}
