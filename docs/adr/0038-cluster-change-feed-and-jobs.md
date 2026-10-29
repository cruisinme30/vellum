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
- Decisions: D161
- Issue: #432, #438
- Pull requests: none yet (delivered as a branch)

:::

## Context

With several replicas behind one Service ([ADR 0037](0037-running-several-replicas.md)),
collections, the book list and the path-to-space list behind permission
checks live in each replica's memory. Only the local file watcher cleared
them, and the space list reloaded every 30 s, so a write on one replica
stayed invisible on the others. The brief wants a change on one replica
visible on all of them within a second (#438). This record collects the
decisions that coordinate replicas through the shared index database.

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

## Alternatives

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

## Consequences

- A change on one replica reaches the others' caches within about 250 ms
  plus the reload; the two-replica test checks under one second.
- Each user-file write costs one extra small transaction on the index, and
  appends serialise on one row. That suits hundreds of writes a second;
  batching announcements can lift it later.
- An announcement lost to a database error after its write succeeded
  leaves other replicas stale until their next full reload. Collections
  and books have no timed reload yet.
- Other caches can follow the same feed: MCP change events (#440) are next.
