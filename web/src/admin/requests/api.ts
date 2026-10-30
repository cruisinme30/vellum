// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The Admin → Requests API: the ring buffer of recent traces kept by the
 * server (`vellum-observe`), served by the admin service.
 */

import { newRequestId, newTraceparent } from "../../platform/telemetry";

/** A `[name, value]` pair, as the server encodes span fields. */
export type Field = readonly [string, string];

/** One row of the request list. */
export interface TraceSummary {
  readonly seq: number;
  readonly trace_id: string;
  readonly started_at_ms: number;
  readonly duration_us: number;
  readonly name: string;
  readonly route?: string;
  readonly method?: string;
  readonly status?: number;
  readonly request_id?: string;
  readonly error: boolean;
  readonly span_count: number;
  readonly has_client: boolean;
}

/** An event logged inside a span. */
export interface SpanEvent {
  readonly offset_us: number;
  readonly level: string;
  readonly message: string;
}

/** A server span. */
interface RecordedSpan {
  readonly index: number;
  /** Absent on the root span. */
  readonly parent?: number;
  readonly name: string;
  readonly target: string;
  readonly start_us: number;
  readonly duration_us: number;
  readonly fields: readonly Field[];
  readonly events: readonly SpanEvent[];
  readonly error: boolean;
}

/** A browser span joined to the trace. */
interface ClientSpanRecord {
  readonly name: string;
  readonly start_us: number;
  readonly duration_us: number;
  readonly attributes: readonly Field[];
}

/** A whole trace. */
export interface Trace {
  readonly trace_id: string;
  readonly started_at_ms: number;
  readonly duration_us: number;
  readonly spans: readonly RecordedSpan[];
  readonly client: readonly ClientSpanRecord[];
  readonly dropped_spans: number;
}

/** Filters for the list; every one is optional. */
export interface RequestFilter {
  readonly route?: string;
  readonly minDurationMs?: number;
  readonly errorsOnly?: boolean;
  readonly id?: string;
  readonly before?: number;
  readonly limit?: number;
}

/** A failed admin call, with the request id for the error dialog. */
export class AdminApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly requestId: string,
  ) {
    super(message);
  }
}

/** The base path of the admin requests API. */
export const REQUESTS_PATH = "/api/v1/admin/requests";

function query(filter: RequestFilter): string {
  const parameters = new URLSearchParams();
  if (filter.route) parameters.set("route", filter.route);
  if (filter.minDurationMs !== undefined)
    parameters.set("min_duration_ms", String(filter.minDurationMs));
  if (filter.errorsOnly) parameters.set("errors_only", "true");
  if (filter.id) parameters.set("id", filter.id);
  if (filter.before !== undefined)
    parameters.set("before", String(filter.before));
  if (filter.limit !== undefined) parameters.set("limit", String(filter.limit));
  const text = parameters.toString();
  return text === "" ? "" : `?${text}`;
}

async function getJson(doFetch: typeof fetch, url: string): Promise<unknown> {
  const requestId = newRequestId();
  let response: Response;
  try {
    response = await doFetch(url, {
      headers: {
        accept: "application/json",
        "x-request-id": requestId,
        traceparent: newTraceparent(),
      },
      credentials: "include",
    });
  } catch (error) {
    throw new AdminApiError(`network: ${String(error)}`, 0, requestId);
  }
  if (!response.ok) {
    throw new AdminApiError(
      `the server answered ${String(response.status)}`,
      response.status,
      requestId,
    );
  }
  return response.json();
}

/** RED and saturation figures for one route, marked against its p95 budget. */
export interface RouteStat {
  readonly method: string;
  readonly route: string;
  readonly rate_per_s: number;
  readonly error_ratio: number;
  readonly p50_ms?: number;
  readonly p95_ms?: number;
  readonly p99_ms?: number;
  readonly in_flight: number;
  readonly counts: { readonly requests: number; readonly errors: number };
  readonly budget_ms?: number;
  readonly over_budget: boolean;
}

/** The routes view: per-route figures over a window. */
export interface RoutesView {
  readonly minutes: number;
  readonly routes: readonly RouteStat[];
  readonly in_flight: number;
}

/** The admin routes API. */
export const ROUTES_PATH = "/api/v1/admin/routes";

/** Per-route figures over the last `minutes`, busiest first. */
export async function fetchRoutes(
  minutes: number,
  doFetch: typeof fetch = globalThis.fetch.bind(globalThis),
): Promise<RoutesView> {
  return (await getJson(
    doFetch,
    `${ROUTES_PATH}?minutes=${String(minutes)}`,
  )) as RoutesView;
}

/** Recent requests, newest first. */
export async function fetchRequests(
  filter: RequestFilter,
  doFetch: typeof fetch = globalThis.fetch.bind(globalThis),
): Promise<TraceSummary[]> {
  const body = (await getJson(doFetch, `${REQUESTS_PATH}${query(filter)}`)) as {
    requests: TraceSummary[];
  };
  return body.requests;
}

/** One trace with every span. */
export async function fetchTrace(
  traceId: string,
  doFetch: typeof fetch = globalThis.fetch.bind(globalThis),
): Promise<Trace> {
  return (await getJson(
    doFetch,
    `${REQUESTS_PATH}/${encodeURIComponent(traceId)}`,
  )) as Trace;
}
