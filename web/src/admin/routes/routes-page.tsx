// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { useEffect, useState } from "react";

import { AdminApiError, fetchRoutes, type RouteStat, type RoutesView } from "../requests/api";

import "../requests/requests.css";

/** The windows an admin can pick, in minutes. */
const WINDOWS = [5, 15, 60] as const;

type Load =
  | { readonly state: "loading" }
  | { readonly state: "ready"; readonly value: RoutesView }
  | { readonly state: "failed"; readonly message: string };

function milliseconds(value: number | undefined): string {
  if (value === undefined) return "–";
  const digits = value < 10 ? 1 : 0;
  return `${value.toFixed(digits)} ms`;
}

function overSummary(over: number): string {
  if (over === 0) return "Every route is within its p95 budget.";
  const subject = over === 1 ? "route is" : "routes are";
  return `${String(over)} ${subject} over the p95 budget.`;
}

function percent(ratio: number): string {
  return `${(ratio * 100).toFixed(ratio > 0 && ratio < 0.01 ? 2 : 1)} %`;
}

function RouteRow({ stat }: { readonly stat: RouteStat }) {
  return (
    <tr className={stat.over_budget ? "vl-requests__row--error" : undefined}>
      <th scope="row">
        {stat.method} {stat.route}
      </th>
      <td>{stat.rate_per_s.toFixed(2)}/s</td>
      <td>{percent(stat.error_ratio)}</td>
      <td>{milliseconds(stat.p50_ms)}</td>
      <td>
        {milliseconds(stat.p95_ms)}
        {stat.over_budget && <strong className="vl-routes__over"> over budget</strong>}
      </td>
      <td>{milliseconds(stat.p99_ms)}</td>
      <td>{stat.budget_ms === undefined ? "–" : `${String(stat.budget_ms)} ms`}</td>
      <td>{stat.in_flight}</td>
    </tr>
  );
}

function RouteTable({ view }: { readonly view: RoutesView }) {
  if (view.routes.length === 0) {
    return <p>No requests in the last {view.minutes} minutes.</p>;
  }
  const over = view.routes.filter((stat) => stat.over_budget).length;
  return (
    <>
      <p role="status">
        {overSummary(over)} {view.in_flight} in flight.
      </p>
      <table className="vl-requests__table">
        <caption>Routes, busiest first, over the last {view.minutes} minutes</caption>
        <thead>
          <tr>
            <th scope="col">Route</th>
            <th scope="col">Rate</th>
            <th scope="col">Errors</th>
            <th scope="col">p50</th>
            <th scope="col">p95</th>
            <th scope="col">p99</th>
            <th scope="col">Budget</th>
            <th scope="col">In flight</th>
          </tr>
        </thead>
        <tbody>
          {view.routes.map((stat) => (
            <RouteRow key={`${stat.method} ${stat.route}`} stat={stat} />
          ))}
        </tbody>
      </table>
    </>
  );
}

/** Admin → Routes: RED and saturation per route, with slow routes marked. */
export function RoutesPage({ fetcher }: { readonly fetcher?: typeof fetch }) {
  const [minutes, setMinutes] = useState<number>(15);
  const [loaded, setLoaded] = useState<{ readonly key: number; readonly load: Load }>();

  useEffect(() => {
    let live = true;
    fetchRoutes(minutes, fetcher).then(
      (value) => {
        if (live) setLoaded({ key: minutes, load: { state: "ready", value } });
      },
      (error: unknown) => {
        const message =
          error instanceof AdminApiError
            ? `${error.message} (request ${error.requestId})`
            : String(error);
        if (live) setLoaded({ key: minutes, load: { state: "failed", message } });
      },
    );
    return () => {
      live = false;
    };
  }, [minutes, fetcher]);

  const load: Load = loaded?.key === minutes ? loaded.load : { state: "loading" };
  return (
    <section className="vl-requests">
      <h1>Routes</h1>
      <label>
        Window{" "}
        <select
          value={minutes}
          onChange={(event) => {
            setMinutes(Number(event.currentTarget.value));
          }}
        >
          {WINDOWS.map((window) => (
            <option key={window} value={window}>
              Last {window} minutes
            </option>
          ))}
        </select>
      </label>
      {load.state === "loading" && <p role="status">Loading routes…</p>}
      {load.state === "failed" && <p role="alert">Could not load routes: {load.message}</p>}
      {load.state === "ready" && <RouteTable view={load.value} />}
    </section>
  );
}
