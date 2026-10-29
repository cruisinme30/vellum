---
id: 01M63E2B54MZ23PY7HD11KE4WM
title: "ADR 0021: Observability, audit log and backups"
status: draft
owner: "@cruisinme30"
tags: [adr, observability, admin, backups]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D91, D92, D93
- Issue: #177, #176, #178, #179, #180, #181, #192
- Pull requests: #254

:::

## Context

Brief §12 asks Vellum to explain itself without an external stack: a
Requests waterfall over recent traces, RED and saturation per route, SLOs
with error budgets, browser timings joined to server traces, and flame
graphs. Brief §11 asks for an append-only audit log and encrypted backups
with a monthly restore drill. All of it runs inside the single binary, so
memory must stay bounded, and none of it may leak workspace content.

## Decisions

### D91: an in-process trace buffer and route metrics in `vellum-observe`

- A `tracing` layer keeps the last 10,000 request traces in a ring buffer
  with a 32 MiB budget, up to 512 spans each; older traces are evicted.
- The gateway calls `RequestObservation` per request: W3C trace context,
  the root span, RED metrics labelled by route template, and a
  `Server-Timing` header that carries the `traceparent` to the browser.
- The same figures go to OpenTelemetry (HTTP semantic conventions) and to
  a 60-minute in-process window, so the console works without a collector.
- Every route has a p95 budget (150 ms for reads, 300 ms for writes,
  matching the SLOs) and is marked over budget once it has 20 requests in
  the window. `vellum budgets` turns this into a CI gate.
- `deploy/observability` ships the collector config, burn-rate alerts and
  Grafana dashboards, all derived from the same SLOs.

### D92: flame graphs from span time, not a CPU sampler

- `vellum profile` folds the span trees of recent traces into stacks
  weighted by self time and renders an SVG in-house.
- Span time shows waiting on the database, storage and locks, which is
  where a Markdown-backed server spends most of its time.
- No sampler means no `unsafe` code, no signals and no extra permissions.

### D93: a sealed, hash-chained audit log and separately keyed backups

- Each audit record holds the SHA-256 of the previous one, is sealed with
  `vellum_crypto::seal`, and is stored as a line in a monthly segment. A
  broken chain refuses to open and the console shows where it broke.
- Settings changes are audited as field diffs; an export is audited too.
- Backups are `.vbk` files: sealed 4 MiB chunks and a sealed manifest
  with every file's hash. They use their own key provider, so a leaked
  backup does not expose the workspace keys.
- The restore drill restores into scratch space, checks every hash and
  fails when it takes longer than the 15-minute RTO.

## Alternatives

- **Require Jaeger or Tempo for traces**: a single binary must explain a
  slow request on its own; the external stack stays optional.
- **A sampling CPU profiler (`pprof`)**: needs `unsafe` signal handling
  and misses time spent waiting, which dominates here.
- **`inferno` for flame graph SVGs**: its CDDL licence is not allowed by
  the dependency policy, and the renderer is small.
- **Audit records in the database**: the index database is rebuilt from
  the files, so the log lives in files with its own chain.

## Consequences

- The console answers "why was this slow?" with no setup, at a fixed cost
  of about 32 MiB.
- Budgets live in admin settings; Prometheus alerts use the defaults, so
  a route with its own budget needs its own alert rule.
- Losing the backup key makes every backup unreadable; the admin guide
  tells operators to store it apart from the data.
- Revisit the buffer size and the span-only profiler after the first load
  tests.
