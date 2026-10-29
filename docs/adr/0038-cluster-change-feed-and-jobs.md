---
id: 01M6475MRMR7MMW2PQWSTC3AHP
title: "ADR 0038: Cluster change feed and background jobs"
status: draft
owner: "@cruisinme30"
tags: [adr, deploy, scale]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D161, D162, D163, D164
- Issue: #432, #438, #439, #63
- Pull requests: none yet (delivered as a branch)

:::

## Context

With several replicas behind one Service ([ADR 0037](0037-running-several-replicas.md)),
collections, the book list and the path-to-space list behind permission
checks live in each replica's memory. Only the local file watcher cleared
them, and the space list reloaded every 30 s, so a write on one replica
stayed invisible on the others. The brief wants a change on one replica
visible on all of them within a second (#438). Background work ran on
every replica too: each pod rescanned the whole workspace at startup, the
search feed swept every page each second, and the calendar sync ran N
times (#439), while notification delivery waited for a queue (#63). This
record collects the decisions that coordinate replicas through the shared
index database.

## Decisions

### `D161`: replicas share a change feed in the index database

- `cluster_feed` and `cluster_changes` (migration 0008) hold an ordered
  list of `(topic, subject, origin)`. Each append bumps the single
  `cluster_feed` row first, as `sync_meta` does, so positions commit in
  order and a reader never steps over one that commits late.
- Serve wraps its file store so every successful write, delete or rename
  of a user file announces its path (topic `path`). The drive announces a
  written space (topic `spaces`) in the same transaction as the space.
- Each replica reads past its cursor every 250 ms, skips its own changes,
  re-reads only the collection folders that changed, forgets the book list
  when a `book.yaml` or `SUMMARY.md` changed, and reloads the space list
  when a space did.
- A targeted collection reload that cannot match a full load (a repeated
  ID, a relation to a collection that went) drops everything instead.
- Positions older than the newest 100,000 are pruned every 5 minutes. A
  reader behind the pruned horizon, or ahead of a rebuilt index, reloads
  every cache. The 30 s space-list reload stays as a backstop.

### `D162`: a durable job queue in the index database

- `jobs` (migration 0010) holds queued work. Postgres claims the oldest
  due rows with `FOR UPDATE SKIP LOCKED`; SQLite, with one writer, claims
  in one statement. A claim is a lease that the worker extends while the
  job runs; when it runs out, another worker takes the job.
- Failures retry with doubling backoff (2 s up to 10 minutes, plus jitter)
  until the attempt limit, then stay `failed` for operators. A permanent
  failure stops at once.
- An idempotency key deduplicates while a job is ready or running. Each
  queue holds at most 100,000 unfinished jobs; past that, enqueue fails.
- Each attempt runs in a `job` span carrying the enqueuing `traceparent`.
  Finished jobs are pruned after a week. Redis is not needed.

### `D163`: run-once work holds a lease, and only the first replica rescans

- `Leases::hold` runs work while a replica holds a named lease, renewing
  it every third of its 30 s TTL and cancelling the work if a renewal finds
  it lost. `singleton` keeps one copy of a loop running in the cluster.
- Every replica beats in `cluster_nodes`. A replica rescans the workspace
  at startup only when no other replica is alive, since a live one has
  kept the index current, and only while holding `drive.rescan`.
- Pruning (the change feed, the page change log, finished jobs) and the
  Google Calendar sync run under their own leases on one replica.
- `cluster_cursors` keeps a lease holder's position, saved only while the
  holder still holds the lease, so the next holder carries on from it.

### `D164`: search follows a page change log instead of sweeping

- Triggers on `drive_nodes` append a page to `drive_changes` when it is
  added, removed, or a searchable field changes, whoever writes it.
- On Postgres each row records its transaction; readers consume only
  transactions older than every running one (`pg_snapshot_xmin`), in
  `(txid, seq)` order, so writers share no lock and no row lands behind a
  cursor. On SQLite the sequence alone is in commit order.
- The feed reads past its cursor each second. With a shared search index,
  one replica runs it under `search.drive_feed` and saves its cursor
  after each indexed batch; with per-replica indexes each follows alone.
- Changes older than a day are pruned; a reader behind them queues every
  page, as a first start does.

## Alternatives

### For the change feed

- **Postgres `LISTEN/NOTIFY`**: lower latency, but it needs a dedicated
  connection that transaction-mode PgBouncer cannot give, loses messages
  while a replica reconnects, and has no SQLite equivalent.
- **Reusing the sync log**: it keeps only the latest version of drive
  records, and collection and book files are written past the drive.
- **Shorter full reloads**: every replica re-reading every collection
  several times a second costs O(workspace) per replica.
- **Querying collections from the index instead of memory**: removes the
  cache, but rewrites the collections engine; kept as a follow-up for
  very large collections.

### For background work

- **Postgres advisory locks for run-once work**: tied to one connection,
  which a pool or PgBouncer may swap, and absent from SQLite. A lease row
  works on both and shows who holds it.
- **A Redis queue**: one more service to run for the HA profiles, when the
  index database already gives `SKIP LOCKED`.
- **A `modified_at` cursor on `drive_nodes`**: misses deletions, and rows
  written by long transactions commit behind the cursor.

## Consequences

### Change feed

- A change on one replica reaches the others' caches within about 250 ms
  plus the reload; the two-replica test checks under one second.
- Each user-file write costs one extra small transaction on the index, and
  appends serialise on one row. That suits hundreds of writes a second;
  batching announcements can lift it later.
- An announcement lost to a database error after its write succeeded
  leaves other replicas stale until their next full reload. Collections
  and books have no timed reload yet.
- Other caches can follow the same feed: MCP change events (#440) are next.

### Background work

- With three replicas, a page change is indexed once and idle search cost
  no longer grows with the number of pages.
- A transaction left open anywhere in the database holds the page log back
  until it ends; indexing resumes then.
- A replica that starts while others run skips the rescan, so files changed
  in a bucket outside Vellum are picked up at the next full start or by
  `vellum reindex`.
- Lease holders can briefly overlap after a network split, so leased work
  and job handlers must be safe to repeat. Delivery jobs come next.
