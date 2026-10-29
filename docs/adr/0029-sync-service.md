---
id: 01M63CXBKW2K1QQGQM2Z11AEJ4
title: "ADR 0029: The sync service behind the local-first clients"
status: verified
owner: "@cruisinme30"
tags: [adr, sync, local-first, clients]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D116, D117, D118
- Builds on: ADR 0023 (D98, the client side of the contract)
- Requirements: R-WK-6 (sub-100 ms perceived latency), R-LT-4 (offline),
  permission changes visible in under 1 s (brief §12)

:::

## Context

ADR 0023 gave the web, PWA and desktop clients a local-first store that
pushes named mutations and pulls record patches. The server half has to
apply each mutation once, tell every client about every change to the
records it may read, and stay correct when permissions change, the index
is rebuilt, or a device stays offline for weeks. It must run on SQLite in
the single binary and on Postgres in HA, and a service may never read
another service's tables.

## Decisions

### D116: the log keeps the latest version of each record

- The change log holds one row per record (space, collection, key) at the
  sequence number of its last write, plus a tombstone for a delete. Every
  write anywhere takes the next sequence number. A pull returns rows with
  a higher number than the cursor, in order, at most 1,000 per page with
  `more: true` when another page waits.
- So the log never grows past the number of records, a client offline for
  a month downloads each changed record once, and a full snapshot is just
  "every live row". A record rewritten while a client pages simply moves
  to a later page.
- Tombstones are pruned after 30 days; the pruning point is the
  **horizon**. A cursor older than the horizon resets.
- Services publish changes made outside a push (REST, GraphQL, MCP, the
  file watcher, other services) through `Sync::publish`, so every path
  that changes a record reaches every client.

### D117: pushes are ordered, at most once, and bound to one account

- A client ID belongs to the first user who pushes with it; another user
  using that ID gets `permission_denied` on push and pull.
- Mutation IDs in a push must increase. The service skips IDs at or below
  the client's last processed ID, then runs the rest in order and records
  the new last ID after each one, so a retried push never applies a
  mutation twice.
- Each mutator belongs to the service that owns the data and is
  registered as `<service>.<verb>`; names are unique. A mutator returns
  the record changes to log. It receives the client ID and mutation ID as
  an idempotency key for its own writes.
- An unknown mutator or a non-retryable error rejects that mutation with
  the kernel error code and processing continues. A retryable error
  (`unavailable`, `timeout`, `rate_limited`) stops the push there; the
  client retries later. At most 500 mutations per push.

### D118: pulls show only what the person may read, and reset on change

- Each record carries its space and, when narrower, the object whose
  permission governs it (a page, an issue). A pull checks all records of a
  page in one batched call to the authorization service and drops the
  rest.
- The cursor is `v1.<log epoch>.<access epoch>.<seq>`. A pull resets
  (`reset: true`, full snapshot of readable records) when the cursor is
  empty or unreadable, when the log epoch changed (index rebuilt), when the
  person's permission epoch changed (they gained or lost access to
  something), or when the cursor is older than the horizon. Revoked
  records thus disappear from devices without the log having to record
  per-person deletes.
- For "visible in under 1 s", the collaboration WebSocket sends a poke
  when a person's permission epoch or a subscribed space changes, and the
  client pulls at once; polling every 30 s is the fallback.
- Store adapters (in memory now; SQLite and Postgres on the index
  database) must pass the shared conformance suite in
  `vellum_sync::conformance`.

## Consequences

- The gateway serves `sync.push` (`POST /api/v1/sync/push`, scope
  `write:direct`) and `sync.pull` (`GET /api/v1/sync/pull`, scope `read`)
  from the operations this crate registers.
- A rebuild of the index bumps the log epoch; every client then
  downloads a fresh snapshot once.
- Permission changes cost one snapshot per affected person, which is rare
  and bounded by what they can read.
