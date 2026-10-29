---
id: 01M64PJ9QGTPYMHM3FPNYGY89C
title: "ADR 0046: Real usage counts and observability wiring"
status: draft
owner: "@cruisinme30"
tags: [adr, observability, admin]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D194, D195, D196, D197
- Issue: #473
- Pull requests: to be linked when merged

:::

## Context

ADR 0021 built the admin console and anonymous telemetry, but the server
never counted anything: the usage report carried zero users, spaces and
documents, and no screen or metric showed how many people use a server or
how much it stores. Brief §12 asks for OpenTelemetry metrics everywhere and
saturation per route. Two instruments that already existed were never
switched on by `vellum serve`: the process memory and thread gauges, and
the blob store meter, so the shipped dashboards had empty panels.

## Decisions

### D194: the owning services count, the composition root combines

- Identity counts people (`Identity::people`): accounts, enabled accounts,
  administrators, unexpired sessions and API tokens, and people seen in
  the last 1, 7 and 30 days from session and token use.
- The drive index counts content (`DriveIndex::totals`): drives, live
  items by kind, the trash, saved versions, bytes of files and versions,
  and activity entries in the last 24 hours and 7 days. One SQL text runs
  on SQLite and Postgres; the memory index answers the same.
- `vellum-cli` joins both into `vellum_admin::counts::UsageCounts`.

### D195: counts are sampled and cached, never computed per scrape

- A `UsageSampler` counts at start-up and every five minutes.
- Gauges and Admin → Usage read the cached copy, so a metrics export or a
  page view never queries the database. `GET /v1/admin/usage?refresh=true`
  counts again on demand; a failed count keeps the last one.

### D196: `vellum.usage.*` gauges through the kernel meter

- `vellum_kernel::telemetry::meter` hands out the process meter.
- Gauges: `vellum.usage.users{state}`, `active_users{window}`, `sessions`,
  `api_tokens`, `spaces`, `items{kind}`, `storage{kind}` (bytes) and
  `activity{window}`. Labels are fixed vocabularies, so cardinality is
  constant. Replicas report the same numbers; dashboards take `max`.
- A "Vellum: usage" Grafana dashboard charts them with live editing rooms
  and blob store rate, latency and bytes.

### D197: exact counts stay on the server

- Only admins see exact counts (`admin.usage`, admin scope).
- Anonymous telemetry keeps sending size buckets: active people in the
  last 30 days, drives, and pages plus books plus collections.
- `vellum serve` now registers the process gauges and wraps the blob
  store in `vellum_store::Metered`.

## Alternatives

- Counting inside each gauge callback: callbacks are synchronous and run
  on every export, so each scrape would hit the database. Rejected.
- Counting from the shared `documents` table: a reindex clears it, while
  the drive's own tables are the drive's view. Rejected.
- An index on `drive_activity.at`: the five-minute count scans the table,
  which is acceptable now; migration numbers are reserved elsewhere, so an
  index can follow if large installs need it.

## Consequences

- The telemetry preview and reports now carry real size buckets.
- Admin → Usage is a new screen; the admin home lists it first.
- Collection rows and Work issues are not yet counted as documents; the
  collections and Work services can add their own counts the same way.
