---
id: 01M63CFE62SKYAP32V5TPN2P58
title: Observability
vellum: 1
---

# Observability

Vellum records OpenTelemetry traces, metrics and logs for every request,
job, collaboration room and MCP call.

## Built in {#built-in}

**Admin → Requests** shows the last 10,000 traces as waterfalls, with
views for the slowest requests, errors and per-route summaries. Each
route shows its request rate, error rate and latency against its budget,
and is marked when it runs over.

Every response carries a `Server-Timing` header and a request ID. Error
dialogs in the app show the request ID with **Copy diagnostic**, so a
user's report leads you straight to the trace.

## Exporting {#export}

Set `OTEL_EXPORTER_OTLP_ENDPOINT` to send everything to an OpenTelemetry
collector. The repository's `deploy/` folder ships Grafana dashboards
and alert rules for the service levels in [Service levels](README.md).

## What is traced {#spans}

Authorization checks, cache lookups, database statements (with row
counts and timings), storage reads and writes, rendering, export,
diagrams, search and collaboration. The W3C `traceparent` header
follows a request across HTTP, WebSockets, background jobs and MCP, so
one trace shows the whole path.

## Profiling {#profiling}

```bash
vellum profile --route /api/pages --seconds 30
```

writes a flame graph of the server while it handles that route.

## Anonymous usage telemetry {#usage}

Vellum can send anonymous usage counts to the project to guide its
development. It is off unless you turn it on in **Admin → Telemetry**,
and the page shows exactly what would be sent.
